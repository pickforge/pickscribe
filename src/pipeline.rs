use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::audio::{self, Level, Recorder, SAMPLE_RATE};
use crate::cleanup;
use crate::config::{self, Config};
use crate::control::Command;
use crate::dictionary::Dictionary;
use crate::history::{self, Item, Mode};
use crate::output;
use crate::review;
use crate::status::{Hub, State, Timings};
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

/// What one dictation produced, for the capsule and the status hub.
#[derive(Default)]
struct Outcome {
    notice: Option<&'static str>,
    timings: Option<Timings>,
    saved: bool,
}

impl Outcome {
    fn notice(notice: &'static str) -> Self {
        Self {
            notice: Some(notice),
            ..Self::default()
        }
    }
}

pub struct Controller {
    pub config: Config,
    pub transcriber: Transcriber,
    pub ui: async_channel::Sender<UiEvent>,
    pub level: Level,
    pub hub: Arc<Hub>,
    pub client: cleanup::Client,
    /// Set while a dictionary review runs in the background.
    pub reviewing: Arc<AtomicBool>,
}

impl Controller {
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
                (Command::Cancel, Some(_)) => {
                    self.show(UiEvent::Hidden);
                    self.hub.finished(None, false);
                }
                (Command::Cancel, None) => {}
            }
        }
    }

    fn start(&self, raw: bool) -> Option<Recording> {
        match Recorder::start(self.level.clone(), self.config.max_recording_secs) {
            Ok(recorder) => {
                self.show(UiEvent::Listening { raw });
                self.hub.set_state(State::Listening, raw);
                if !raw && config::api_key().is_some() {
                    self.client.warm_up();
                }
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
        let stopped = Instant::now();
        self.show(UiEvent::Thinking { raw: recording.raw });
        self.hub.set_state(State::Thinking, recording.raw);
        let pcm = recording.recorder.finish();
        let outcome = self.process(&pcm, recording.raw, stopped);
        self.show(match outcome.notice {
            Some(notice) => UiEvent::Notice(notice.into()),
            None => UiEvent::Hidden,
        });
        self.hub.finished(outcome.timings, outcome.saved);
    }

    fn process(&self, pcm: &[f32], raw: bool, stopped: Instant) -> Outcome {
        if pcm.len() < MIN_SAMPLES || audio::peak_window_rms(pcm) < MIN_SPEECH_RMS {
            return Outcome::notice(NO_SPEECH);
        }
        let dictionary = Dictionary::load().unwrap_or_else(|err| {
            eprintln!("pickscribe: {err:#}");
            Dictionary::default()
        });
        let started = Instant::now();
        let (transcript, doubtful) = match self.transcriber.transcribe(pcm, &dictionary.terms()) {
            Ok(heard) if heard.text.is_empty() => return Outcome::notice(NO_SPEECH),
            Ok(heard) => (dictionary.apply(&heard.text), heard.doubtful),
            Err(err) => {
                eprintln!("pickscribe: transcription failed: {err:#}");
                return Outcome::notice("Transcription failed");
            }
        };
        let transcribe_ms = millis(started.elapsed());
        let started = Instant::now();
        let (text, mode, notice) = if raw {
            (transcript.clone(), Mode::Raw, None)
        } else {
            self.cleaned(&dictionary, &transcript)
        };
        let cleanup_ms = (!raw).then(|| millis(started.elapsed()));
        if text.is_empty() {
            return Outcome::notice(NO_SPEECH);
        }
        let audio_ms = pcm.len() as u64 * 1000 / u64::from(SAMPLE_RATE);
        let mut item = Item::new(mode, text, transcript, audio_ms);
        item.doubtful = doubtful;
        let notice = match output::deliver(&item.text) {
            Ok(()) => {
                item.delivered = true;
                notice
            }
            Err(err) => {
                eprintln!("pickscribe: paste failed: {err:#}");
                Some("Paste failed · text copied")
            }
        };
        let timings = Timings {
            audio_ms,
            transcribe_ms,
            cleanup_ms,
            total_ms: millis(stopped.elapsed()),
            mode,
        };
        eprintln!(
            "pickscribe: {audio_ms} ms of audio, transcribed in {transcribe_ms} ms, cleanup {} ms, pasted after {} ms",
            cleanup_ms.unwrap_or(0),
            timings.total_ms
        );
        let saved = self.save(&item);
        if saved && mode == Mode::Ai {
            self.review_later();
        }
        Outcome {
            notice,
            timings: Some(timings),
            saved,
        }
    }

    /// Starts a background dictionary review when enough AI dictations piled up.
    fn review_later(&self) {
        if self.config.review_every == 0 || self.reviewing.swap(true, Ordering::SeqCst) {
            return;
        }
        let client = self.client.clone();
        let config = self.config.clone();
        let hub = Arc::clone(&self.hub);
        let reviewing = Arc::clone(&self.reviewing);
        std::thread::spawn(move || {
            review::run_due(&client, &config, &hub);
            reviewing.store(false, Ordering::SeqCst);
        });
    }

    /// Cleanup problems never lose the dictation: the raw transcript is pasted instead.
    fn cleaned(
        &self,
        dictionary: &Dictionary,
        transcript: &str,
    ) -> (String, Mode, Option<&'static str>) {
        let Some(key) = config::api_key() else {
            return (
                transcript.to_string(),
                Mode::Raw,
                Some("No API key · raw text pasted"),
            );
        };
        match self
            .client
            .clean(&self.config, dictionary, &key, transcript)
        {
            Ok(text) => (dictionary.apply(&text), Mode::Ai, None),
            Err(err) => {
                eprintln!("pickscribe: cleanup failed: {err:#}");
                (
                    transcript.to_string(),
                    Mode::Raw,
                    Some("Cleanup failed · raw text pasted"),
                )
            }
        }
    }

    fn save(&self, item: &Item) -> bool {
        if self.config.history_limit == 0 {
            return false;
        }
        match history::append(item, self.config.history_limit) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("pickscribe: cannot save history: {err:#}");
                false
            }
        }
    }

    fn show(&self, event: UiEvent) {
        let _ = self.ui.send_blocking(event);
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
