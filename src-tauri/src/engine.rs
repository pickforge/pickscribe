use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pickscribe::config::AppConfig;
use pickscribe::engine::{cleanup, levels::LevelMeter, paste, recorder, sounds, stt};
use pickscribe::history::{HistoryDb, HistoryEntry, NewEntry};
use pickscribe::platform;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT_STATE: &str = "pickscribe://state";
pub const EVENT_LEVEL: &str = "pickscribe://level";
pub const EVENT_HISTORY: &str = "pickscribe://history";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Idle,
    Recording,
    Transcribing,
    Cleaning,
    Pasting,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatePayload {
    pub stage: Stage,
    /// Unix millis when the active recording started.
    pub recording_started_ms: Option<u64>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub last_entry: Option<HistoryEntry>,
}

pub struct Engine {
    recording: Mutex<Option<recorder::Recording>>,
    state: Mutex<StatePayload>,
    levels_running: Arc<AtomicBool>,
    /// Paste chord requested by the triggering invocation (e.g. the legacy
    /// terminal wrapper forwards --paste-chord=ctrl-shift-v). The last toggle
    /// wins; None falls back to the configured chord.
    chord_override: Mutex<Option<String>>,
    pub history: HistoryDb,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Engine {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            recording: Mutex::new(None),
            state: Mutex::new(StatePayload {
                stage: Stage::Idle,
                recording_started_ms: None,
                message: None,
                error: None,
                last_entry: None,
            }),
            levels_running: Arc::new(AtomicBool::new(false)),
            chord_override: Mutex::new(None),
            history: HistoryDb::open_default()?,
        })
    }

    pub fn set_chord_override(&self, chord: Option<String>) {
        *self.chord_override.lock().unwrap() = chord;
    }

    pub fn state(&self) -> StatePayload {
        self.state.lock().unwrap().clone()
    }

    fn set_state(&self, app: &AppHandle, update: impl FnOnce(&mut StatePayload)) {
        let payload = {
            let mut state = self.state.lock().unwrap();
            update(&mut state);
            state.clone()
        };
        let _ = app.emit(EVENT_STATE, &payload);
        crate::tray::sync(app, payload.stage);
    }

    pub fn toggle(self: &Arc<Self>, app: &AppHandle) {
        let stage = self.state.lock().unwrap().stage;
        match stage {
            Stage::Idle => self.start(app),
            Stage::Recording => self.stop(app),
            // Ignore toggles while the pipeline is busy.
            _ => {}
        }
    }

    pub fn start(self: &Arc<Self>, app: &AppHandle) {
        let support = platform::current();
        if let Some(message) = support.unsupported_dictation_message() {
            if AppConfig::load().general.sounds {
                sounds::play(sounds::Cue::Error);
            }
            self.set_state(app, |s| {
                s.stage = Stage::Idle;
                s.recording_started_ms = None;
                s.error = Some(message);
                s.message = None;
            });
            return;
        }

        let cfg = AppConfig::load();
        let recording = match recorder::start(&cfg.stt) {
            Ok(rec) => rec,
            Err(err) => {
                if cfg.general.sounds {
                    sounds::play(sounds::Cue::Error);
                }
                self.set_state(app, |s| {
                    s.stage = Stage::Idle;
                    s.error = Some(format!("{err:#}"));
                    s.message = None;
                });
                return;
            }
        };
        if cfg.general.sounds {
            sounds::play(sounds::Cue::Start);
        }
        let audio_path = recording.audio_path.clone();
        *self.recording.lock().unwrap() = Some(recording);
        self.set_state(app, |s| {
            s.stage = Stage::Recording;
            s.recording_started_ms = Some(now_ms());
            s.error = None;
            s.message = None;
        });

        // Live level meter for the waveform.
        self.levels_running.store(true, Ordering::SeqCst);
        let running = Arc::clone(&self.levels_running);
        let level_app = app.clone();
        std::thread::spawn(move || {
            let mut meter: Option<LevelMeter> = None;
            while running.load(Ordering::SeqCst) {
                if meter.is_none() {
                    meter = LevelMeter::open(&audio_path).ok();
                }
                let level = meter.as_mut().and_then(|m| m.poll()).unwrap_or(0.0);
                let _ = level_app.emit(EVENT_LEVEL, level);
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = level_app.emit(EVENT_LEVEL, 0.0f32);
        });
    }

    pub fn stop(self: &Arc<Self>, app: &AppHandle) {
        let Some(recording) = self.recording.lock().unwrap().take() else {
            return;
        };
        self.levels_running.store(false, Ordering::SeqCst);
        let cfg = AppConfig::load();
        if cfg.general.sounds {
            sounds::play(sounds::Cue::Stop);
        }
        self.set_state(app, |s| {
            s.stage = Stage::Transcribing;
            s.recording_started_ms = None;
        });

        let engine = Arc::clone(self);
        let app = app.clone();
        std::thread::spawn(move || engine.run_pipeline(&app, cfg, recording));
    }

    pub fn cancel(self: &Arc<Self>, app: &AppHandle) {
        self.levels_running.store(false, Ordering::SeqCst);
        if let Some(recording) = self.recording.lock().unwrap().take() {
            recording.cancel();
        }
        self.set_state(app, |s| {
            s.stage = Stage::Idle;
            s.recording_started_ms = None;
            s.message = Some("Recording cancelled".into());
            s.error = None;
        });
    }

    fn run_pipeline(
        self: Arc<Self>,
        app: &AppHandle,
        cfg: AppConfig,
        recording: recorder::Recording,
    ) {
        let fail = |err: String| {
            if cfg.general.sounds {
                sounds::play(sounds::Cue::Error);
            }
            self.set_state(app, |s| {
                s.stage = Stage::Idle;
                s.error = Some(err);
            });
        };

        let (audio_path, duration_ms) = match recording.stop() {
            Ok(result) => result,
            Err(err) => return fail(format!("{err:#}")),
        };

        let raw = match stt::transcribe(&cfg.stt, &audio_path) {
            Ok(text) => text,
            Err(err) => {
                if !cfg.general.keep_audio {
                    let _ = fs::remove_file(&audio_path);
                }
                return fail(format!("{err:#}"));
            }
        };
        if !cfg.general.keep_audio {
            let _ = fs::remove_file(&audio_path);
        }
        if raw.is_empty() {
            self.set_state(app, |s| {
                s.stage = Stage::Idle;
                s.message = Some("No speech detected".into());
                s.error = None;
            });
            return;
        }

        self.set_state(app, |s| s.stage = Stage::Cleaning);
        let outcome = cleanup::clean(&cfg, &raw);

        self.set_state(app, |s| s.stage = Stage::Pasting);
        let mut paste_cfg = cfg.paste.clone();
        if let Some(chord) = self.chord_override.lock().unwrap().clone() {
            paste_cfg.chord = chord;
        }
        let paste_error = paste::deliver(&paste_cfg, &outcome.text)
            .err()
            .map(|err| format!("paste failed (text copied if possible): {err:#}"));

        let entry = NewEntry {
            duration_ms: duration_ms as i64,
            raw_text: raw.clone(),
            cleaned_text: if outcome.cleaned {
                Some(outcome.text.clone())
            } else {
                None
            },
            provider: outcome.provider.clone(),
            model: outcome.model.clone(),
            language: cfg.stt.language.clone(),
        };
        let last_entry = match self.history.insert(&entry) {
            Ok(id) => {
                let _ = app.emit(EVENT_HISTORY, ());
                Some(HistoryEntry {
                    id,
                    created_at: (now_ms() / 1000) as i64,
                    duration_ms: entry.duration_ms,
                    raw_text: entry.raw_text,
                    cleaned_text: entry.cleaned_text,
                    provider: entry.provider,
                    model: entry.model,
                    language: entry.language,
                    word_count: pickscribe::history::count_words(&outcome.text),
                })
            }
            Err(_) => None,
        };

        self.set_state(app, |s| {
            s.stage = Stage::Idle;
            s.error = paste_error.or(outcome.error.clone());
            s.message = Some(if outcome.cleaned {
                "Cleaned and pasted".into()
            } else {
                "Pasted raw transcript".into()
            });
            s.last_entry = last_entry;
        });
    }
}

pub fn engine(app: &AppHandle) -> Arc<Engine> {
    Arc::clone(&*app.state::<Arc<Engine>>())
}
