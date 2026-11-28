//! Audio output that follows the system's output device.
//!
//! rodio's `OutputStream` opens one device once: when that device goes away (headphones or a
//! USB/Bluetooth headset unplugged, Android's AAudio stream disconnected by a route change), its
//! stream stops and nothing reopens it, so the game stays silent until it is restarted; when the
//! system's default device changes (Windows), the sound also stays on the old device.
//!
//! Here the mix (rodio's dynamic mixer, with every voice already playing) lives apart from the
//! device stream. A small thread owns the cpal stream: it reopens it on the current default device
//! as soon as the stream reports an error, and when the default device changes (checked once per
//! second, the thread sleeps in between). The mix keeps its format (the one of the device opened
//! first, exactly as rodio chose it); a later device with another rate or channel count gets a
//! conversion, so music and sounds carry on where they were. Only the output changes: the voices
//! and everything the game computes are untouched.

use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::{self, FromSample, Sample, SizedSample};
use rodio::dynamic_mixer::{self, DynamicMixer, DynamicMixerController};
use rodio::Source;
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Period of the default-device check and minimal delay between two opening attempts.
const RETRY: Duration = Duration::from_secs(1);

/// Channel count and sample rate of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub channels: u16,
    pub rate: u32,
}

/// Mix shared by the successive device streams (None until the first device is open).
type Shared = Arc<Mutex<Option<DynamicMixer<f32>>>>;

enum Event {
    /// The stream reported an error (device lost, disconnected, invalidated).
    Error,
    Quit,
}

/// Output device handling, independent of cpal so that it can be tested.
trait Backend {
    type Stream;
    /// Identity of the system's default output device (None: none, or not known).
    fn default_device(&mut self) -> Option<String>;
    /// Opens a stream on the default device (or another one if it fails); returns it with the
    /// identity of the default device at that moment.
    fn open(&mut self) -> Option<(Self::Stream, String)>;
}

/// What a step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Nothing to do (stream fine, or retry not due yet).
    Idle,
    /// A stream was opened.
    Opened,
    /// No stream could be opened (retried after RETRY).
    Failed,
}

/// Keeps a stream open on the current default device.
struct Follower<B: Backend> {
    backend: B,
    current: Option<(B::Stream, String)>,
    last_attempt: Option<Instant>,
}

impl<B: Backend> Follower<B> {
    fn new(backend: B) -> Self {
        Follower { backend, current: None, last_attempt: None }
    }

    /// One step: `error` = the stream reported an error since the last step. The stream is
    /// dropped on error or when the default device changed, then reopened (at most once per
    /// RETRY).
    fn step(&mut self, error: bool, now: Instant) -> Step {
        if error {
            // The stream is dead: dropped at once.
            self.current = None;
        }
        if let Some((_, id)) = &self.current {
            match self.backend.default_device() {
                // Default device unknown for now: the open stream is kept.
                None => return Step::Idle,
                Some(d) if d == *id => return Step::Idle,
                Some(_) => {}
            }
        }
        if self.last_attempt.is_some_and(|t| now.duration_since(t) < RETRY) {
            // Retry not due: a still working stream plays on meanwhile.
            return Step::Idle;
        }
        self.last_attempt = Some(now);
        // The old stream is closed before the new one opens (exclusive devices).
        self.current = None;
        match self.backend.open() {
            Some(s) => {
                self.current = Some(s);
                Step::Opened
            }
            None => Step::Failed,
        }
    }
}

/// Conversion of the mix to a device's format: linear interpolation between frames, channels
/// copied (a mono device gets their average, extra channels stay silent).
struct Conv {
    from: Format,
    to: Format,
    /// Input frames per output frame.
    step: f64,
    /// Position between `a` and `b` (0…1).
    pos: f64,
    a: Vec<f32>,
    b: Vec<f32>,
    primed: bool,
    frame: Vec<f32>,
}

impl Conv {
    fn new(from: Format, to: Format) -> Conv {
        let n = from.channels.max(1) as usize;
        Conv {
            from,
            to,
            step: from.rate as f64 / to.rate.max(1) as f64,
            pos: 0.0,
            a: vec![0.0; n],
            b: vec![0.0; n],
            primed: false,
            frame: vec![0.0; n],
        }
    }

    fn same(&self) -> bool {
        self.from == self.to
    }

    fn read(src: &mut impl Iterator<Item = f32>, into: &mut [f32]) {
        for v in into.iter_mut() {
            *v = src.next().unwrap_or(0.0);
        }
    }

    /// Fills `out` (interleaved in the device's format) from the mix.
    fn fill<T: Sample + FromSample<f32>>(&mut self, src: &mut impl Iterator<Item = f32>, out: &mut [T]) {
        if self.same() {
            // Same format as the mix: samples passed as they are (as rodio's own output does).
            for d in out.iter_mut() {
                *d = src.next().map(T::from_sample).unwrap_or(T::EQUILIBRIUM);
            }
            return;
        }
        if !self.primed {
            Self::read(src, &mut self.a);
            Self::read(src, &mut self.b);
            self.primed = true;
        }
        let inc = self.from.channels.max(1) as usize;
        let outc = self.to.channels.max(1) as usize;
        for f in out.chunks_mut(outc) {
            while self.pos >= 1.0 {
                std::mem::swap(&mut self.a, &mut self.b);
                Self::read(src, &mut self.b);
                self.pos -= 1.0;
            }
            let t = self.pos as f32;
            for c in 0..inc {
                self.frame[c] = self.a[c] + (self.b[c] - self.a[c]) * t;
            }
            self.pos += self.step;
            let mono = (outc == 1).then(|| self.frame.iter().sum::<f32>() / inc as f32);
            for (c, d) in f.iter_mut().enumerate() {
                let v = match mono {
                    Some(m) => m,
                    None if c < inc => self.frame[c],
                    None if inc == 1 => self.frame[0],
                    None => 0.0,
                };
                *d = T::from_sample(v);
            }
        }
    }
}

/// cpal device handling.
struct Cpal {
    shared: Shared,
    /// Format of the mix (that of the first device opened).
    root: Option<Format>,
    events: Sender<Event>,
    /// Sends the mixer input to `Output::new` once the first device is open.
    ready: Option<std::sync::mpsc::SyncSender<Option<(Arc<DynamicMixerController<f32>>, Format)>>>,
}

/// All the supported output configurations of a device, in rodio's order (fallback when the
/// default configuration does not open).
fn supported_formats(device: &cpal::Device) -> Vec<cpal::SupportedStreamConfig> {
    let hz_44100 = cpal::SampleRate(44_100);
    let Ok(list) = device.supported_output_configs() else { return Vec::new() };
    let mut supported: Vec<_> = list.collect();
    supported.sort_by(|a, b| b.cmp_default_heuristics(a));
    supported
        .into_iter()
        .flat_map(|sf| {
            let (max, min) = (sf.max_sample_rate(), sf.min_sample_rate());
            let mut v = vec![sf.with_max_sample_rate()];
            if hz_44100 < max && hz_44100 > min {
                v.push(sf.with_sample_rate(hz_44100));
            }
            v.push(sf.with_sample_rate(min));
            v
        })
        .collect()
}

impl Cpal {
    fn build<T: SizedSample + FromSample<f32>>(&self, device: &cpal::Device, config: &cpal::SupportedStreamConfig) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        f32: FromSample<T>,
    {
        let to = Format { channels: config.channels(), rate: config.sample_rate().0 };
        let mut conv = Conv::new(self.root.unwrap_or(to), to);
        let shared = self.shared.clone();
        let events = self.events.clone();
        device.build_output_stream::<T, _, _>(
            &config.config(),
            move |data: &mut [T], _| {
                let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
                match g.as_mut() {
                    Some(mix) => conv.fill(mix, data),
                    None => data.fill(T::EQUILIBRIUM),
                }
            },
            move |err| {
                eprintln!("sound: output error: {err}");
                let _ = events.send(Event::Error);
            },
            None,
        )
    }

    fn try_config(&mut self, device: &cpal::Device, config: &cpal::SupportedStreamConfig) -> Option<cpal::Stream> {
        use cpal::SampleFormat as F;
        let stream = match config.sample_format() {
            F::F32 => self.build::<f32>(device, config),
            F::F64 => self.build::<f64>(device, config),
            F::I8 => self.build::<i8>(device, config),
            F::I16 => self.build::<i16>(device, config),
            F::I32 => self.build::<i32>(device, config),
            F::I64 => self.build::<i64>(device, config),
            F::U8 => self.build::<u8>(device, config),
            F::U16 => self.build::<u16>(device, config),
            F::U32 => self.build::<u32>(device, config),
            F::U64 => self.build::<u64>(device, config),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        let to = Format { channels: config.channels(), rate: config.sample_rate().0 };
        let name = device.name().unwrap_or_default();
        println!("sound: output {name}: {} Hz, {} channels, {:?}, buffer {:?}", to.rate, to.channels, config.sample_format(), config.buffer_size());
        if self.root.is_none() {
            // First device: the mix takes its format (as rodio's output did).
            self.root = Some(to);
            let (ctl, mix) = dynamic_mixer::mixer::<f32>(to.channels, to.rate);
            *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = Some(mix);
            if let Some(r) = self.ready.take() {
                let _ = r.send(Some((ctl, to)));
            }
        } else if self.root != Some(to) {
            println!("sound: mix converted from {:?}", self.root);
        }
        Some(stream)
    }

    /// Default configuration first, then the others (rodio's order).
    fn try_device(&mut self, device: &cpal::Device) -> Option<cpal::Stream> {
        if let Ok(c) = device.default_output_config() {
            if let Some(s) = self.try_config(device, &c) {
                return Some(s);
            }
        }
        supported_formats(device).iter().find_map(|c| self.try_config(device, c))
    }
}

impl Backend for Cpal {
    type Stream = cpal::Stream;

    fn default_device(&mut self) -> Option<String> {
        cpal::default_host().default_output_device()?.name().ok()
    }

    fn open(&mut self) -> Option<(cpal::Stream, String)> {
        let host = cpal::default_host();
        let default = host.default_output_device();
        let id = default.as_ref().and_then(|d| d.name().ok()).unwrap_or_default();
        if let Some(s) = default.as_ref().and_then(|d| self.try_device(d)) {
            return Some((s, id));
        }
        // Default device unusable: any other one (as rodio's try_default).
        let others = host.output_devices().ok()?;
        for d in others {
            if let Some(s) = self.try_device(&d) {
                return Some((s, id));
            }
        }
        None
    }
}

/// Device thread: opens the first stream, then follows errors and the default device.
fn run(backend: Cpal, rx: Receiver<Event>) {
    // Created here: cpal streams stay on the thread that opened them.
    let mut follower = Follower::new(backend);
    if follower.step(false, Instant::now()) != Step::Opened {
        // No device at all at startup: no output (as before).
        if let Some(r) = follower.backend.ready.take() {
            let _ = r.send(None);
        }
        return;
    }
    loop {
        let error = match rx.recv_timeout(RETRY) {
            Ok(Event::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Event::Error) => true,
            Err(RecvTimeoutError::Timeout) => false,
        };
        // Errors already queued belong to the same stream.
        let mut error = error;
        while let Ok(e) = rx.try_recv() {
            match e {
                Event::Quit => return,
                Event::Error => error = true,
            }
        }
        if error {
            println!("sound: output lost, reopening on the default device");
        }
        let had = follower.current.is_some();
        match follower.step(error, Instant::now()) {
            Step::Opened if had && !error => println!("sound: default output device changed"),
            Step::Failed if error || had => println!("sound: no output device for now, retrying every second"),
            _ => {}
        }
    }
}

/// Audio output: sources played into a mix that survives device changes.
pub struct Output {
    mixer: Arc<DynamicMixerController<f32>>,
    format: Format,
    events: Sender<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Output {
    /// Opens the default output device (None if there is no usable device at all).
    pub fn new() -> Option<Output> {
        let (events, rx) = channel();
        let (ready_tx, ready_rx) = sync_channel(1);
        let backend = Cpal { shared: Arc::new(Mutex::new(None)), root: None, events: events.clone(), ready: Some(ready_tx) };
        let thread = std::thread::Builder::new().name("sound output".into()).spawn(move || run(backend, rx)).ok()?;
        match ready_rx.recv() {
            Ok(Some((mixer, format))) => Some(Output { mixer, format, events, thread: Some(thread) }),
            _ => {
                let _ = thread.join();
                None
            }
        }
    }

    /// Format of the mix (that of the first device).
    #[allow(dead_code)]
    pub fn format(&self) -> Format {
        self.format
    }

    /// Plays a source until it ends.
    pub fn play_raw<S: Source<Item = f32> + Send + 'static>(&self, source: S) {
        self.mixer.add(source);
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake system: a default device that can change or disappear, opening that can fail.
    struct Fake {
        default: Option<String>,
        can_open: bool,
        opens: u32,
    }

    impl Backend for Fake {
        type Stream = u32;
        fn default_device(&mut self) -> Option<String> {
            self.default.clone()
        }
        fn open(&mut self) -> Option<(u32, String)> {
            if !self.can_open {
                return None;
            }
            self.opens += 1;
            Some((self.opens, self.default.clone().unwrap_or_default()))
        }
    }

    fn follower() -> Follower<Fake> {
        Follower::new(Fake { default: Some("speakers".into()), can_open: true, opens: 0 })
    }

    #[test]
    fn reopens_after_error() {
        let t0 = Instant::now();
        let mut f = follower();
        assert_eq!(f.step(false, t0), Step::Opened);
        assert_eq!(f.step(false, t0 + RETRY * 2), Step::Idle);
        // Earbuds pulled out: the stream reports an error, it is reopened at once.
        assert_eq!(f.step(true, t0 + RETRY * 3), Step::Opened);
        assert_eq!(f.current.as_ref().map(|c| c.0), Some(2));
    }

    #[test]
    fn follows_default_device() {
        let t0 = Instant::now();
        let mut f = follower();
        f.step(false, t0);
        f.backend.default = Some("headphones".into());
        assert_eq!(f.step(false, t0 + RETRY), Step::Opened);
        assert_eq!(f.current.as_ref().map(|c| c.1.as_str()), Some("headphones"));
        // Default unknown for a moment: the stream is kept.
        f.backend.default = None;
        assert_eq!(f.step(false, t0 + RETRY * 2), Step::Idle);
        assert!(f.current.is_some());
    }

    #[test]
    fn retries_without_device_at_most_once_per_period() {
        let t0 = Instant::now();
        let mut f = follower();
        f.step(false, t0);
        f.backend.can_open = false;
        assert_eq!(f.step(true, t0 + RETRY), Step::Failed);
        assert!(f.current.is_none());
        // Repeated errors or ticks before RETRY: no new attempt.
        assert_eq!(f.step(true, t0 + RETRY + RETRY / 2), Step::Idle);
        f.backend.can_open = true;
        assert_eq!(f.step(false, t0 + RETRY * 2), Step::Opened);
    }

    /// 200 random sequences of plugs, unplugs, errors and ticks: after every step the stream
    /// is either open on the current default device, or a retry is due within RETRY; never
    /// more than one attempt per RETRY.
    #[test]
    fn random_sequences() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = move |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) % n
        };
        for _ in 0..200 {
            let t0 = Instant::now();
            let mut f = follower();
            let mut now = t0;
            let mut attempts: Vec<Instant> = Vec::new();
            for _ in 0..100 {
                now += Duration::from_millis(100 + rnd(1500) as u64);
                match rnd(5) {
                    0 => f.backend.default = Some(["speakers", "headphones", "usb"][rnd(3) as usize].into()),
                    1 => f.backend.can_open = rnd(3) != 0,
                    _ => {}
                }
                let error = rnd(6) == 0;
                let before = f.backend.opens;
                let step = f.step(error, now);
                if step != Step::Idle {
                    attempts.push(now);
                }
                if step == Step::Opened {
                    assert_eq!(f.backend.opens, before + 1);
                }
                if let Some((_, id)) = &f.current {
                    // On the current default device, or a retry due within RETRY.
                    let pending = f.last_attempt.is_some_and(|t| now.duration_since(t) < RETRY);
                    assert!(Some(id) == f.backend.default.as_ref() || pending);
                } else {
                    assert!(step != Step::Opened);
                }
            }
            for w in attempts.windows(2) {
                assert!(w[1].duration_since(w[0]) >= RETRY);
            }
        }
    }

    /// Negative control: a follower that never checks the default device misses a change.
    #[test]
    fn negative_control_default_change_detected() {
        let t0 = Instant::now();
        let mut f = follower();
        f.step(false, t0);
        f.backend.default = Some("headphones".into());
        f.step(false, t0 + RETRY);
        assert_eq!(f.backend.opens, 2, "the change of default device must reopen the stream");
    }

    /// Silent source counting the samples the device pulls.
    struct Counter(Arc<std::sync::atomic::AtomicUsize>);
    impl Iterator for Counter {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(0.0)
        }
    }
    impl Source for Counter {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            44_100
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    /// Real output device (needs one; silent): a stream error reopens the device and the same
    /// voice keeps being played. `cargo test -p openxiii-viewer real_device -- --ignored`
    #[test]
    #[ignore]
    fn real_device_reopens_after_error() {
        let out = Output::new().expect("no output device");
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        out.play_raw(Counter(n.clone()));
        let pulled = |n: &Arc<std::sync::atomic::AtomicUsize>| {
            let a = n.load(std::sync::atomic::Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(500));
            n.load(std::sync::atomic::Ordering::Relaxed) - a
        };
        assert!(pulled(&n) > 1000, "the device does not play");
        for _ in 0..3 {
            out.events.send(Event::Error).unwrap();
            std::thread::sleep(RETRY + Duration::from_millis(200));
            assert!(pulled(&n) > 1000, "no sound after reopening");
        }
    }

    fn sine(frames: usize, channels: usize, rate: f32, hz: f32) -> Vec<f32> {
        (0..frames * channels).map(|i| ((i / channels) as f32 * hz / rate * std::f32::consts::TAU).sin() * 0.5).collect()
    }

    #[test]
    fn same_format_is_passthrough() {
        let f = Format { channels: 2, rate: 44_100 };
        let src = sine(1000, 2, 44_100.0, 440.0);
        let mut out = vec![0f32; 2000];
        Conv::new(f, f).fill(&mut src.clone().into_iter(), &mut out);
        assert_eq!(out, src);
        // i16 device: same conversion as rodio's.
        let mut out16 = vec![0i16; 2000];
        Conv::new(f, f).fill(&mut src.clone().into_iter(), &mut out16);
        assert!(out16.iter().zip(&src).all(|(a, b)| *a == i16::from_sample(*b)));
    }

    /// Zero crossings of the left channel, to compare pitches.
    fn crossings(v: &[f32], channels: usize) -> usize {
        v.chunks(channels).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    #[test]
    fn resampled_keeps_pitch_and_channels() {
        for (to_rate, to_ch) in [(48_000u32, 2u16), (22_050, 2), (48_000, 1), (48_000, 6), (96_000, 2)] {
            let from = Format { channels: 2, rate: 44_100 };
            let to = Format { channels: to_ch, rate: to_rate };
            let secs = 1.0;
            let mut src: Vec<f32> = sine((44_100.0 * secs) as usize + 10, 2, 44_100.0, 440.0);
            // Right channel silent: checks that channels are not swapped.
            for f in src.chunks_mut(2) {
                f[1] = 0.0;
            }
            let n = (to_rate as f32 * secs) as usize;
            let mut out = vec![0f32; n * to_ch as usize];
            Conv::new(from, to).fill(&mut src.into_iter(), &mut out);
            let c = crossings(&out, to_ch as usize);
            assert!((870..=890).contains(&c), "{to_rate} Hz {to_ch} ch: {c} crossings");
            if to_ch >= 2 {
                assert!(out.chunks(to_ch as usize).all(|f| f[1..].iter().all(|v| *v == 0.0)));
            }
        }
    }

    /// The mix survives a change of device: a voice started on the first device carries on,
    /// sample after sample, on the next one.
    #[test]
    fn mix_survives_device_change() {
        use rodio::buffer::SamplesBuffer;
        let fmt = Format { channels: 2, rate: 44_100 };
        let (ctl, mut mix) = dynamic_mixer::mixer::<f32>(2, 44_100);
        let voice: Vec<f32> = (0..20_000).map(|i| (i as f32 * 0.001).sin()).collect();
        ctl.add(SamplesBuffer::new(2, 44_100, voice.clone()));
        let mut first = vec![0f32; 4_000];
        Conv::new(fmt, fmt).fill(&mut mix, &mut first);
        // New device, same format: the voice continues where it stopped.
        let mut second = vec![0f32; 4_000];
        Conv::new(fmt, fmt).fill(&mut mix, &mut second);
        assert_eq!(&first[..], &voice[..4_000]);
        assert_eq!(&second[..], &voice[4_000..8_000]);
    }
}
