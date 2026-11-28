//! Video player: UVideoPlayer (natives 476 to 484 of Engine.dll) and WinDrv.dll's
//! UPCVideoPlayerDevice, which reads the .bik files with binkw32.
//!
//! binkw32.dll is a 32-bit DLL that this program cannot load: the frames are
//! decoded by ffmpeg (OXIII_FFMPEG, otherwise the one next to the program, otherwise "ffmpeg" from the
//! PATH: see ffmpeg()) into RGB565, the format of D3DDrv's
//! texture (Video_Init: D3DFMT_R5G6B5, 1024 × 512). The rest follows the original code:
//! states, frame rate (BinkWait), end of the video, sound track depending on the language.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex, OnceLock};

use crate::value::{ObjId, Value};
use crate::Vm;

/// What UPCVideoPlayerDevice::Tick displayed during the frame (Present outside the game's rendering).
#[derive(Debug, Clone)]
pub enum VideoPresent {
    /// Video_CopyTexToScreen: the frame (RGB565, little-endian) stretched over the whole screen, with
    /// texture coordinates 0..width/1024 and 0..height/512.
    Frame { width: u32, height: u32, rgb565: Arc<Vec<u8>> },
    /// End of the video: black Clear then Present, twice.
    Black,
}

/// Sound of the video (binkw32 plays it itself through DirectSound, synchronized with the frames).
#[derive(Debug, Clone)]
pub enum VideoAudio {
    /// First frame decoded: track `track` (rank among the file's tracks) starts.
    Start { path: PathBuf, track: usize },
    /// BinkPause(1) / BinkPause(0).
    Pause(bool),
    /// BinkSetVolume(bink, 0, volume): 32768 = normal volume.
    Volume(i32),
    /// BinkClose.
    Stop,
}

/// Header of a .bik (BINK: Width, Height, Frames, FrameRate, FrameRateDiv, NumTracks).
#[derive(Debug, Clone)]
pub struct BinkHeader {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub frame_rate: u32,
    pub frame_rate_div: u32,
    /// Identifiers of the sound tracks (BinkSetSoundTrack designates them by identifier).
    pub track_ids: Vec<u32>,
}

impl BinkHeader {
    pub fn read(data: &[u8]) -> Option<Self> {
        if data.len() < 44 || &data[..3] != b"BIK" {
            return None;
        }
        let u = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let tracks = u(40)? as usize;
        // Tracks: maximum sizes (4 bytes each), frequency and flags (4 bytes), then
        // the identifiers.
        let ids_at = 44 + tracks * 8;
        let track_ids = (0..tracks).map(|i| u(ids_at + i * 4)).collect::<Option<Vec<_>>>()?;
        Some(BinkHeader { width: u(20)?, height: u(24)?, frames: u(8)?, frame_rate: u(28)?, frame_rate_div: u(32)?.max(1), track_ids })
    }
}

/// Open HBINK: header, current frame, decoder.
#[derive(Debug)]
struct Bink {
    path: PathBuf,
    header: BinkHeader,
    /// FrameNum (+0xC): next frame to decode, starting at 1.
    frame_num: u32,
    /// ReadError (+0x1C).
    read_error: bool,
    paused: bool,
    /// Rank of the chosen sound track (BinkSetSoundTrack).
    track: usize,
    /// Playback time (Bink's clock, stopped while paused) and time of the first frame.
    clock: f64,
    started: bool,
    decoder: Option<Decoder>,
}

#[derive(Debug)]
struct Decoder {
    child: Child,
    frames: Receiver<Vec<u8>>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// ffmpeg program, looked up once: OXIII_FFMPEG, otherwise ffmpeg(.exe) next to the program
/// (standalone XIII.exe shipped with its own; Android: android.rs sets OXIII_FFMPEG to the
/// APK's libffmpeg.so), otherwise "ffmpeg" from the PATH.
fn ffmpeg() -> &'static Path {
    static FFMPEG: OnceLock<PathBuf> = OnceLock::new();
    FFMPEG.get_or_init(|| find_ffmpeg(openxiii_package::env::var_os("OXIII_FFMPEG"), std::env::current_exe().ok().as_deref()))
}

fn find_ffmpeg(variable: Option<std::ffi::OsString>, program: Option<&Path>) -> PathBuf {
    if let Some(v) = variable.filter(|v| !v.is_empty()) {
        return v.into();
    }
    let name_value = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(neighbor) = program.and_then(Path::parent).map(|d| d.join(name_value)).filter(|p| p.is_file()) {
        return neighbor;
    }
    "ffmpeg".into()
}

pub(crate) fn command(args: &[&str]) -> Command {
    let mut c = Command::new(ffmpeg());
    c.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window opens.
        c.creation_flags(0x0800_0000);
    }
    c
}

impl Decoder {
    /// RGB565 frames in order, read by a separate thread.
    fn start(path: &Path, header: &BinkHeader) -> Option<Self> {
        let file = path.to_str()?;
        let mut child = command(&["-v", "error", "-i", file, "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb565le", "-sws_dither", "none", "-"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let size = (header.width * header.height * 2) as usize;
        let (tx, rx) = sync_channel(4);
        std::thread::spawn(move || loop {
            let mut buf = vec![0u8; size];
            if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                return;
            }
        });
        Some(Decoder { child, frames: rx })
    }
}

/// Decoder of a sound track of a .bik: 16-bit stereo PCM at 44,100 Hz (interleaved) on the
/// process's standard output.
pub fn audio_decoder(path: &Path, track: usize) -> Option<Child> {
    let file = path.to_str()?;
    let map = format!("0:a:{track}");
    command(&["-v", "error", "-i", file, "-map", &map, "-f", "s16le", "-ac", "2", "-ar", "44100", "-"]).stdout(Stdio::piped()).spawn().ok()
}

/// UPCVideoPlayerDevice (UVideoPlayerDevice::Instance).
#[derive(Debug, Clone, Default)]
pub struct VideoDevice {
    /// GSys->VideoPath ([Core.System] VideoPath, relative to the System folder).
    pub dir: PathBuf,
    /// VideoPath as written in the ini ("..\Video"), for the log.
    pub path_text: String,
    /// Log lines for the VM to pick up ("Load Video : …").
    pub log: Vec<String>,
    /// +0x2C: 1 during playback (uiGetStatus, IsPlaying).
    pub status: u32,
    /// +0x30: HBINK.
    bink: Option<Arc<Mutex<Bink>>>,
    /// +0x34: width > 512.
    pub wide: bool,
    /// UGameEngine::Draw and UClient::Tick skipped in this frame (status 1 at render time).
    pub draw_skipped: bool,
    /// Last Present of Tick, for the display layer to pick up.
    pub presented: Option<VideoPresent>,
    /// Commands for the video's sound, for the display layer to pick up.
    pub audio: Vec<VideoAudio>,
    /// Tests: empty frames instead of ffmpeg.
    #[cfg(test)]
    pub(crate) false_decoder: bool,
    /// Track passed by UVideoPlayer::execSetSoundTrack; BinkSetSoundTrack(1, &n) only takes effect
    /// at the next BinkOpen (no script calls it: a single sound track per video).
    pub sound_track: i32,
}

impl VideoDevice {
    pub fn new(dir: PathBuf) -> Self {
        VideoDevice { dir, ..Default::default() }
    }

    /// UPCVideoPlayerDevice::Open: VideoPath\Name.bik, sound track of the language.
    pub fn open(&mut self, name: &str, language: &str) -> bool {
        let path = self.dir.join(format!("{name}.bik"));
        // "Load Video : %s" (NAME_Log): VideoPath\Name.bik.
        let shown = self.path_text.clone();
        self.log.push(format!("Log: Load Video : {shown}\\{name}.bik"));
        if self.bink.is_some() {
            self.stop();
        }
        let lang = language.to_ascii_lowercase();
        let mut id = if lang.starts_with("frt") {
            1
        } else if lang.starts_with("det") {
            2
        } else if lang.starts_with("est") {
            4
        } else if lang.starts_with("itt") {
            3
        } else {
            0
        };
        let Some(header) = std::fs::read(&path).ok().and_then(|d| BinkHeader::read(&d)) else {
            return false;
        };
        // BinkOpen a first time to count the tracks: fewer than two, track 0.
        if header.track_ids.len() < 2 {
            id = 0;
        }
        // BinkSetSoundTrack(1, &id) then BinkOpen(BINKSNDTRACK): the track with this identifier
        // (none if the file has none).
        let track = header.track_ids.iter().position(|&t| t == id).unwrap_or(usize::MAX);
        self.wide = header.width > 0x200;
        self.bink =
            Some(Arc::new(Mutex::new(Bink { path, header, frame_num: 1, read_error: false, paused: false, track, clock: 0.0, started: false, decoder: None })));
        true
    }

    /// A video is open (between Open and Stop).
    pub fn is_open(&self) -> bool {
        self.bink.is_some()
    }

    /// Play.
    pub fn play(&mut self) {
        self.status = 1;
    }

    /// Stop: BinkSetSoundOnOff(0), status 0, BinkClose, Video_Close.
    pub fn stop(&mut self) {
        if self.bink.take().is_some() {
            self.status = 0;
            self.audio.push(VideoAudio::Stop);
        }
    }

    /// Pause / Resume: BinkPause.
    pub fn pause(&mut self, pause: bool) {
        if let Some(b) = &self.bink {
            if let Ok(mut b) = b.lock() {
                b.paused = pause;
            }
            self.audio.push(VideoAudio::Pause(pause));
        }
    }

    /// SetSoundTrack: BinkSetSoundTrack(1, &n) only takes effect at the next BinkOpen.
    pub fn set_sound_track(&mut self, track: i32) {
        self.sound_track = track;
    }

    /// SetSoundVolume: BinkSetVolume(bink, 0, volume).
    pub fn set_sound_volume(&mut self, volume: i32) {
        if self.bink.is_some() {
            self.audio.push(VideoAudio::Volume(volume));
        }
    }

    /// UPCVideoPlayerDevice::Tick, called by UGameEngine::Tick (and by XIII.exe's loop
    /// of startup videos): `dt` advances Bink's clock.
    pub fn tick(&mut self, dt: f64) {
        self.tick_with(dt, None);
    }

    /// Tick; `wait_value` forces the result of BinkWait (tests: Bink stubbed), otherwise the clock decides.
    fn tick_with(&mut self, dt: f64, wait_value: Option<bool>) {
        if self.status == 0 {
            return;
        }
        let Some(b) = self.bink.clone() else { return };
        let Ok(mut b) = b.lock() else { return };
        if !b.paused {
            b.clock += dt;
        }
        if b.frame_num == b.header.frames || b.read_error {
            // Last frame reached (it is never decoded): black screen, then Stop.
            drop(b);
            self.presented = Some(VideoPresent::Black);
            self.stop();
            return;
        }
        // BinkWait: frame N is due (N − 1) × FrameRateDiv / FrameRate seconds after the
        // first one; while paused, Bink waits.
        let due = (b.frame_num - 1) as f64 * b.header.frame_rate_div as f64 / b.header.frame_rate.max(1) as f64;
        if wait_value.unwrap_or(b.paused || (b.started && b.clock < due)) {
            return;
        }
        #[cfg(test)]
        let false_value = self.false_decoder;
        #[cfg(not(test))]
        let false_value = false;
        if !b.started && !false_value {
            // First frame: the clock starts from here, and so does the sound.
            b.started = true;
            b.clock = 0.0;
            let header = b.header.clone();
            b.decoder = Decoder::start(&b.path, &header);
            if b.track != usize::MAX {
                self.audio.push(VideoAudio::Start { path: b.path.clone(), track: b.track });
            }
        }
        // BinkDoFrame then BinkNextFrame, BinkCopyToBuffer and Video_CopyTexToScreen.
        let frame = b.decoder.as_ref().and_then(|d| d.frames.recv().ok());
        #[cfg(test)]
        let frame = if false_value { Some(Vec::new()) } else { frame };
        b.started = true;
        b.frame_num += 1;
        match frame {
            Some(pixels) => {
                self.presented = Some(VideoPresent::Frame { width: b.header.width, height: b.header.height, rgb565: Arc::new(pixels) });
            }
            None => b.read_error = true,
        }
    }
}

/// XIIIMenu's sVideo: the main menu's New game video.
const NEW_GAME_VIDEO: &str = "cine00";

impl Vm {
    /// UGameEngine::Tick: VideoPlayerDevice->Tick after rendering (skipped during playback).
    pub(crate) fn video_tick(&mut self, dt: f32) {
        self.engine.video.draw_skipped = self.engine.video.status == 1;
        self.engine.video.tick(dt as f64);
    }

    /// Status of the player (UVideoPlayerDevice::GetStatus).
    pub fn video_status(&self) -> u32 {
        self.engine.video.status
    }

    /// Opens then plays a video like XIII.exe's startup loop (Open, Play).
    pub fn video_open_play(&mut self, name: &str) -> bool {
        let language = self.config.as_ref().map(|c| c.language.clone()).unwrap_or_else(|| "int".into());
        let ok = self.engine.video.open(name, &language);
        let lines = std::mem::take(&mut self.engine.video.log);
        self.log.extend(lines);
        self.engine.video.play();
        ok
    }

    /// Improved build, network co-op: the main menu's New game video (XIIIMenu's sVideo) played
    /// on this machine once both machines met, the sounds stopped first as the menu does
    /// (StopAllSounds). False: the video could not be opened.
    pub fn play_new_game_video(&mut self) -> bool {
        self.audio_dispatch(crate::canvas::AudioCall::Stop { actor: None, sound: None, kinds: 0xfff, notify: true });
        if self.video_open_play(NEW_GAME_VIDEO) {
            return true;
        }
        self.engine.video.stop();
        self.engine.video.status = 0;
        false
    }

    /// The video playing stopped (a player skips it).
    pub fn video_skip(&mut self) {
        self.engine.video.stop();
    }

    /// Tick of the player alone (startup video loop, without the engine).
    pub fn video_tick_only(&mut self, dt: f32) {
        self.engine.video.tick(dt as f64);
    }

    /// ALevelInfo::execGetAvailableRes → UD3DRenderDevice::GetAvailableRes
    /// 24- or 32-bit modes (R8G8B8, A8R8G8B8, X8R8G8B8) in 4:3, from 640 × 480
    /// to 1600 × 1200, without duplicates, in the adapter's order.
    pub fn available_res(&self) -> Vec<(u32, u32)> {
        available_res_of(&self.display_modes)
    }

    /// Calls of the UVideoPlayer natives to the video device (UVideoPlayerDevice::Instance, `engine.video` here); in tests, a stub
    /// records them and returns programmed values.
    fn video_device(&mut self, op: VideoOp) -> i32 {
        match op {
            VideoOp::Open(file) => {
                let language = self.config.as_ref().map(|c| c.language.clone()).unwrap_or_else(|| "int".into());
                let ok = self.engine.video.open(file, &language);
                let lines = std::mem::take(&mut self.engine.video.log);
                self.log.extend(lines);
                ok as i32
            }
            VideoOp::Play => {
                self.engine.video.play();
                0
            }
            VideoOp::Stop => {
                self.engine.video.stop();
                0
            }
            VideoOp::Pause => {
                self.engine.video.pause(true);
                0
            }
            VideoOp::Resume => {
                self.engine.video.pause(false);
                0
            }
            VideoOp::SoundTrack(n) => {
                self.engine.video.set_sound_track(n);
                0
            }
            VideoOp::SoundVolume(n) => {
                self.engine.video.set_sound_volume(n);
                0
            }
            VideoOp::Status => self.engine.video.status as i32,
        }
    }

    /// UVideoPlayer natives: `None` if `name` is not one of them.
    pub(crate) fn video_native(&mut self, name: &str, this: ObjId, args: &mut [Value]) -> Option<Value> {
        if name == "GetAvailableRes" && self.is_a(this, "LevelInfo") {
            let items = self.available_res().into_iter().map(|(w, h)| Value::Struct("levelinfo.resinfo".into(), vec![Value::Int(w as i32), Value::Int(h as i32)].into())).collect();
            if let Some(a) = args.first_mut() {
                *a = Value::Array(items);
            }
            return Some(Value::Void);
        }
        if !matches!(name, "Open" | "Play" | "Stop" | "Pause" | "Resume" | "SetSoundVolume" | "SetSoundTrack" | "GetStatus") || !self.is_a(this, "VideoPlayer")
        {
            return None;
        }
        let int_arg = |a: &[Value]| a.first().map(|a| a.as_int()).unwrap_or(0);
        Some(match name {
            // execOpen: vtable +0x74.
            "Open" => {
                let file = args.first().map(|a| a.as_name()).unwrap_or_default();
                Value::Bool(self.video_device(VideoOp::Open(&file)) != 0)
            }
            // execPlay: vtable +0x78; execStop: +0x80; execPause: +0x84; execResume: +0x88;
            // execSetSoundTrack: +0x8C; execSetSoundVolume: +0x90; all on UVideoPlayerDevice::Instance.
            "Play" => {
                self.video_device(VideoOp::Play);
                Value::Void
            }
            "Stop" => {
                self.video_device(VideoOp::Stop);
                Value::Void
            }
            "Pause" => {
                self.video_device(VideoOp::Pause);
                Value::Void
            }
            "Resume" => {
                self.video_device(VideoOp::Resume);
                Value::Void
            }
            "SetSoundVolume" => {
                self.video_device(VideoOp::SoundVolume(int_arg(args)));
                Value::Void
            }
            "SetSoundTrack" => {
                self.video_device(VideoOp::SoundTrack(int_arg(args)));
                Value::Void
            }
            // execGetStatus: Instance+0x2C.
            "GetStatus" => Value::Int(self.video_device(VideoOp::Status)),
            _ => unreachable!(),
        })
    }
}

/// Operations of the UVideoPlayer natives on the video device.
enum VideoOp<'a> {
    Open(&'a str),
    Play,
    Stop,
    Pause,
    Resume,
    SoundTrack(i32),
    SoundVolume(i32),
    Status,
}

#[cfg(test)]
mod tests_ffmpeg {
    use super::find_ffmpeg;
    use std::path::{Path, PathBuf};

    /// Search order: OXIII_FFMPEG, then ffmpeg(.exe) next to the program, then the PATH.
    #[test]
    fn order_search_ffmpeg() {
        let folder = std::env::temp_dir().join(format!("oxiii-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join("XIII.exe");
        let neighbor = folder.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
        // Nothing next to it: the one from the PATH.
        let _ = std::fs::remove_file(&neighbor);
        assert_eq!(find_ffmpeg(None, Some(&program)), PathBuf::from("ffmpeg"));
        assert_eq!(find_ffmpeg(None, None), PathBuf::from("ffmpeg"));
        // A folder with the same name is not a program.
        std::fs::create_dir_all(&neighbor).unwrap();
        assert_eq!(find_ffmpeg(None, Some(&program)), PathBuf::from("ffmpeg"));
        std::fs::remove_dir(&neighbor).unwrap();
        // The file next to the program.
        std::fs::write(&neighbor, b"").unwrap();
        assert_eq!(find_ffmpeg(None, Some(&program)), neighbor);
        // Empty variable: ignored; variable set: it wins.
        assert_eq!(find_ffmpeg(Some("".into()), Some(&program)), neighbor);
        assert_eq!(find_ffmpeg(Some("C:/tools/ffmpeg.exe".into()), Some(&program)), Path::new("C:/tools/ffmpeg.exe"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}

/// GetAvailableRes's list (see `Vm::available_res`) for these display modes (width, height, bits).
pub fn available_res_of(display_modes: &[(u32, u32, u32)]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &(w, h, bits) in display_modes {
        if (bits == 24 || bits == 32) && w * 3 == h * 4 && w > 0x27f && h > 0x1df && w < 0x641 && h < 0x4b1 && !out.contains(&(w, h)) {
            out.push((w, h));
        }
    }
    out
}
