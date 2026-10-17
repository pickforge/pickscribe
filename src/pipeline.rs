use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::audio::{self, Level, Recorder, SAMPLE_RATE};
use crate::cleanup;
use crate::config::{self, Config};
use crate::control::Command;
use crate::output;
use crate::transcribe::Transcriber;

/// What the floating capsule shows.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Listening { raw: bool },
    Thinking { raw: bool },
    Hidden,
    Notice(String),
}

/// Recordings quieter than this hold no speech; Whisper would invent text for them.
const MIN_SPEECH_RMS: f32 = 0.008;
const MIN_SAMPLES: usize = SAMPLE_RATE as usize / 4;
const NO_SPEECH: &str = "No speech heard";

struct Recording {
    recorder: Recorder,
    raw: bool,
    started: Instant,
}

pub struct Controller {
    config: Config,
    transcriber: Transcriber,
    ui: async_channel::Sender<UiEvent>,
    level: Level,
}

impl Controller {
    pub fn new(
        config: Config,
        transcriber: Transcriber,
        ui: async_channel::Sender<UiEvent>,
        level: Level,
    ) -> Self {
        Self {
            config,
            transcriber,
            ui,
            level,
        }
    }

    pub fn run(self, commands: Receiver<Command>) {
        let limit = Duration::from_secs(self.config.max_recording_secs);
        let mut active: Option<Recording> = None;
        loop {
            let command = match &active {
                // Reaching the length limit stops the recording as if the key was tapped.
                Some(recording) => {
                    match commands.recv_timeout(limit.saturating_sub(recording.started.elapsed())) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => Command::Toggle { raw: recording.raw },
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match commands.recv() {
                    Ok(command) => command,
                    Err(_) => return,
                },
            };
            match (command, active.take()) {
                (Command::Toggle { raw }, None) => active = self.start(raw),
                (Command::Toggle { .. }, Some(recording)) => {
                    self.finish(recording);
                    // Taps made while the text was processed are stale.
                    while commands.try_recv().is_ok() {}
                }
                (Command::Cancel, Some(_)) => self.show(UiEvent::Hidden),
                (Command::Cancel, None) => {}
            }
        }
    }

    fn start(&self, raw: bool) -> Option<Recording> {
        match Recorder::start(self.level.clone(), self.config.max_recording_secs) {
            Ok(recorder) => {
                self.show(UiEvent::Listening { raw });
                Some(Recording {
                    recorder,
                    raw,
                    started: Instant::now(),
                })
            }
            Err(err) => {
                eprintln!("pickscribe: {err:#}");
                self.show(UiEvent::Notice("Microphone unavailable".into()));
                None
            }
        }
    }

    fn finish(&self, recording: Recording) {
        self.show(UiEvent::Thinking { raw: recording.raw });
        let pcm = recording.recorder.finish();
        let event = match self.process(&pcm, recording.raw) {
            Some(notice) => UiEvent::Notice(notice.into()),
            None => UiEvent::Hidden,
        };
        self.show(event);
    }

    /// Returns a notice for the capsule when something needs the user's attention.
    fn process(&self, pcm: &[f32], raw: bool) -> Option<&'static str> {
        if pcm.len() < MIN_SAMPLES || audio::peak_window_rms(pcm) < MIN_SPEECH_RMS {
            return Some(NO_SPEECH);
        }
        let transcript = match self.transcriber.transcribe(pcm) {
            Ok(text) if text.is_empty() => return Some(NO_SPEECH),
            Ok(text) => text,
            Err(err) => {
                eprintln!("pickscribe: transcription failed: {err:#}");
                return Some("Transcription failed");
            }
        };
        let (text, notice) = if raw {
            (transcript, None)
        } else {
            self.cleaned(transcript)
        };
        if text.is_empty() {
            return Some(NO_SPEECH);
        }
        if let Err(err) = output::deliver(&text) {
            eprintln!("pickscribe: paste failed: {err:#}");
            return Some("Paste failed · text copied");
        }
        notice
    }

    /// Cleanup problems never lose the dictation: the raw transcript is pasted instead.
    fn cleaned(&self, transcript: String) -> (String, Option<&'static str>) {
        let Some(key) = config::api_key() else {
            return (transcript, Some("No API key · raw text pasted"));
        };
        match cleanup::clean(&self.config, &key, &transcript) {
            Ok(text) => (text, None),
            Err(err) => {
                eprintln!("pickscribe: cleanup failed: {err:#}");
                (transcript, Some("Cleanup failed · raw text pasted"))
            }
        }
    }

    fn show(&self, event: UiEvent) {
        let _ = self.ui.send_blocking(event);
    }
}
