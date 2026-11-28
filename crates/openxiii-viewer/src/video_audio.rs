//! Video sound: binkw32 plays the chosen track through its own DirectSound output, next to the
//! HX engine; it carries on from one map to the next like the player.

use openxiii_vm::video::VideoAudio;
use crate::audio_output::Output;
use rodio::{Sink, Source};
use std::io::Read;
use std::process::Child;
use std::sync::mpsc::{sync_channel, Receiver};
use std::time::Duration;

/// Samples read as the decoder goes.
struct Pipe {
    rx: Receiver<Vec<i16>>,
    block: Vec<i16>,
    at: usize,
}

impl Iterator for Pipe {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        while self.at >= self.block.len() {
            self.block = self.rx.recv().ok()?;
            self.at = 0;
        }
        self.at += 1;
        Some(self.block[self.at - 1])
    }
}

impl Source for Pipe {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub struct VideoSound {
    /// Output that follows the system's device (reopened when headphones are unplugged, etc.).
    output: Output,
    playing: Option<(Sink, Child)>,
}

impl VideoSound {
    pub fn new() -> Option<Self> {
        Some(VideoSound { output: Output::new()?, playing: None })
    }

    fn stop(&mut self) {
        if let Some((sink, mut child)) = self.playing.take() {
            sink.stop();
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// The sound stopped if one is playing (its video is no longer open).
    pub fn stop_if_playing(&mut self) {
        if self.playing.is_some() {
            eprintln!("video sound: stopped, its video is no longer open");
            self.stop();
        }
    }

    /// Applies the player's commands for the frame.
    pub fn apply(&mut self, calls: Vec<VideoAudio>) {
        for call in calls {
            match call {
                VideoAudio::Start { path, track } => {
                    self.stop();
                    let Some(mut child) = openxiii_vm::video::audio_decoder(&path, track) else { continue };
                    let Some(mut out) = child.stdout.take() else { continue };
                    let (tx, rx) = sync_channel(64);
                    std::thread::spawn(move || loop {
                        let mut buf = vec![0u8; 8192];
                        let mut n = 0;
                        while n < buf.len() {
                            match out.read(&mut buf[n..]) {
                                Ok(0) | Err(_) => break,
                                Ok(k) => n += k,
                            }
                        }
                        let samples: Vec<i16> = buf[..n & !1].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                        let end = n < buf.len();
                        if (!samples.is_empty() && tx.send(samples).is_err()) || end {
                            return;
                        }
                    });
                    let (sink, queue) = Sink::new_idle();
                    self.output.play_raw(queue);
                    sink.append(Pipe { rx, block: Vec::new(), at: 0 });
                    self.playing = Some((sink, child));
                }
                VideoAudio::Pause(pause) => {
                    if let Some((sink, _)) = &self.playing {
                        if pause {
                            sink.pause();
                        } else {
                            sink.play();
                        }
                    }
                }
                // BinkSetVolume: 32768 = normal volume (up to 65536, double).
                VideoAudio::Volume(v) => {
                    if let Some((sink, _)) = &self.playing {
                        sink.set_volume(v.clamp(0, 65536) as f32 / 32768.0);
                    }
                }
                VideoAudio::Stop => self.stop(),
            }
        }
    }
}
