use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
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
    lock: LockMonitor,
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
            let command = match self.next_command(&commands, &mut active, limit) {
                Ok(Some(command)) => command,
                Ok(None) => continue,
                Err(_) => return,
            };
            match (command, active.take()) {
                (Command::Toggle { raw }, None) => active = self.start(raw),
                (Command::Toggle { raw }, Some(mut recording)) => {
                    recording.raw = raw_at_either_tap(recording.raw, raw);
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

    fn next_command(
        &self,
        commands: &Receiver<Command>,
        active: &mut Option<Recording>,
        limit: Duration,
    ) -> Result<Option<Command>, mpsc::RecvError> {
        let Some(recording) = active else {
            return commands.recv().map(Some);
        };
        if recording.lock.locked() {
            active.take();
            self.show(UiEvent::Notice("Session locked".into()));
            self.hub.finished(None, false);
            return Ok(None);
        }
        match commands.recv_timeout(
            limit
                .saturating_sub(recording.started.elapsed())
                .min(Duration::from_secs(1)),
        ) {
            Ok(command) => Ok(Some(command)),
            // Reaching the length limit stops the recording as if the key was tapped.
            Err(RecvTimeoutError::Timeout) if recording.started.elapsed() >= limit => {
                Ok(Some(Command::Toggle { raw: recording.raw }))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
        }
    }

    fn start(&self, raw: bool) -> Option<Recording> {
        let lock = LockMonitor::start(session_locked);
        match Recorder::start(self.level.clone(), self.config.max_recording_secs) {
            Ok(recorder) => {
                self.show(UiEvent::Listening { raw });
                self.hub.set_state(State::Listening, raw);
                if !raw && config::api_key().is_some() {
                    self.client.warm_up();
                }
                Some(Recording {
                    recorder,
                    lock,
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

    fn finish(&self, mut recording: Recording) {
        let stopped = Instant::now();
        let pcm = recording.recorder.finish();
        let focus = output::Focus::capture();
        if recording.lock.stop() {
            self.show(UiEvent::Notice("Session locked".into()));
            self.hub.finished(None, false);
            return;
        }
        self.show(UiEvent::Thinking { raw: recording.raw });
        self.hub.set_state(State::Thinking, recording.raw);
        let outcome = self.process(&pcm, recording.raw, stopped, &focus);
        self.show(match outcome.notice {
            Some(notice) => UiEvent::Notice(notice.into()),
            None => UiEvent::Hidden,
        });
        self.hub.finished(outcome.timings, outcome.saved);
    }

    fn process(&self, pcm: &[f32], raw: bool, stopped: Instant, focus: &output::Focus) -> Outcome {
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
        let delivery = output::deliver(&item.text, focus, session_locked);
        item.delivered = delivery == output::Delivery::Pasted;
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
        let notice = delivery.notice(saved).or(notice);
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
        let reset = ReviewGuard(Arc::clone(&self.reviewing));
        let client = self.client.clone();
        let config = self.config.clone();
        let hub = Arc::clone(&self.hub);
        std::thread::spawn(move || {
            let _reset = reset;
            review::run_due(&client, &config, &hub);
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
            Ok(text) => cleaned_text(dictionary, transcript, &text),
            Err(err) => {
                eprintln!("pickscribe: cleanup failed: {err:#}");
                let notice = if err.is::<cleanup::Answered>() {
                    "Cleanup answered instead · raw text pasted"
                } else {
                    "Cleanup failed · raw text pasted"
                };
                (transcript.to_string(), Mode::Raw, Some(notice))
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

fn raw_at_either_tap(start: bool, stop: bool) -> bool {
    start || stop
}

fn cleaned_text(
    dictionary: &Dictionary,
    transcript: &str,
    text: &str,
) -> (String, Mode, Option<&'static str>) {
    if text.trim().is_empty() {
        (
            transcript.to_string(),
            Mode::Raw,
            Some("Cleanup empty · raw text pasted"),
        )
    } else {
        (dictionary.apply(text), Mode::Ai, None)
    }
}

struct ReviewGuard(Arc<AtomicBool>);

impl Drop for ReviewGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Checks beside the microphone startup and remembers any lock until recording ends.
struct LockMonitor {
    locked: Arc<AtomicBool>,
    stop: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LockMonitor {
    fn start(check: impl Fn() -> bool + Send + 'static) -> Self {
        let locked = Arc::new(AtomicBool::new(false));
        let state = Arc::clone(&locked);
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            loop {
                if check() {
                    state.store(true, Ordering::SeqCst);
                    return;
                }
                if receiver.recv_timeout(Duration::from_secs(1)) != Err(RecvTimeoutError::Timeout) {
                    return;
                }
            }
        });
        Self {
            locked,
            stop,
            worker: Some(worker),
        }
    }

    fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    fn stop(&mut self) -> bool {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            // A failed check must not allow delivery from an unchecked recording.
            if worker.join().is_err() {
                self.locked.store(true, Ordering::SeqCst);
            }
        }
        self.locked()
    }
}

impl Drop for LockMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock_value(program: &str, args: &[&str]) -> Option<bool> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_lock_value(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn parse_lock_value(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "yes" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

fn session_locked() -> bool {
    session_locked_from(lock_value("omarchy-shell", &["lock", "isLocked"]), || {
        let session = logind_session(std::env::var("XDG_SESSION_ID").ok());
        lock_value(
            "loginctl",
            &["show-session", &session, "--property=LockedHint", "--value"],
        )
    })
}

fn logind_session(session: Option<String>) -> String {
    session
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "auto".into())
}

fn session_locked_from(omarchy: Option<bool>, logind: impl FnOnce() -> Option<bool>) -> bool {
    omarchy.or_else(logind).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_at_either_tap_skips_cleanup() {
        for (start, stop, raw) in [
            (false, false, false),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            assert_eq!(raw_at_either_tap(start, stop), raw);
        }
    }

    #[test]
    fn empty_cleanup_keeps_the_transcript() {
        let (text, mode, notice) = cleaned_text(&Dictionary::default(), "hello", " ");
        assert_eq!(text, "hello");
        assert_eq!(mode, Mode::Raw);
        assert!(notice.is_some());
    }

    #[test]
    fn review_flag_resets_on_return_and_panic() {
        let flag = Arc::new(AtomicBool::new(true));
        {
            let _guard = ReviewGuard(Arc::clone(&flag));
        }
        assert!(!flag.load(Ordering::SeqCst));
        flag.store(true, Ordering::SeqCst);
        let _ = std::panic::catch_unwind(|| {
            let _guard = ReviewGuard(Arc::clone(&flag));
            panic!("review failed");
        });
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn lock_checks_prefer_omarchy_and_allow_unreadable_state() {
        assert!(session_locked_from(Some(true), || panic!(
            "unexpected fallback"
        )));
        assert!(!session_locked_from(Some(false), || panic!(
            "unexpected fallback"
        )));
        assert!(session_locked_from(None, || Some(true)));
        assert!(!session_locked_from(None, || Some(false)));
        assert!(!session_locked_from(None, || None));
    }

    #[test]
    fn lock_replies_accept_both_services() {
        assert_eq!(parse_lock_value("true\n"), Some(true));
        assert_eq!(parse_lock_value("yes\n"), Some(true));
        assert_eq!(parse_lock_value("false"), Some(false));
        assert_eq!(parse_lock_value("no"), Some(false));
        assert_eq!(parse_lock_value(""), None);
        assert_eq!(parse_lock_value("error"), None);
    }

    #[test]
    fn logind_uses_auto_without_a_session_id() {
        assert_eq!(logind_session(None), "auto");
        assert_eq!(logind_session(Some(String::new())), "auto");
        assert_eq!(logind_session(Some("7".into())), "7");
    }

    #[test]
    fn lock_monitor_starts_without_waiting_and_checks_before_finishing() {
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut monitor = LockMonitor::start(move || {
            entered.send(()).unwrap();
            wait.recv().unwrap()
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!monitor.locked());
        release.send(true).unwrap();
        assert!(monitor.stop());
        assert!(monitor.locked());
    }

    #[test]
    fn lock_monitor_cancels_when_the_session_locks_during_recording() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let checks = Arc::clone(&calls);
        let (checked, replies) = mpsc::channel();
        let mut monitor = LockMonitor::start(move || {
            let locked = checks.fetch_add(1, Ordering::SeqCst) > 0;
            checked.send(locked).unwrap();
            locked
        });
        assert!(!replies.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(replies.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(monitor.stop());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
