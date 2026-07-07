use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pickscribe::config::AppConfig;
use pickscribe::engine::{
    audio_segments, cleanup,
    incremental::{self, CancelToken},
    levels::LevelMeter,
    paste, recorder,
    segments::{RecordingSession, TranscriptSegment, TranscriptSegmentStatus},
    sounds, stt,
};
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
    pub segments: Vec<TranscriptSegment>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub last_entry: Option<HistoryEntry>,
}

struct ActiveRecording {
    session_id: String,
    cancel_token: CancelToken,
    recording: recorder::Recording,
    incremental: Option<ActiveIncremental>,
}

struct ActiveIncremental {
    worker_cancel_token: CancelToken,
    stop_requested: Arc<AtomicBool>,
    final_duration_ms: Arc<AtomicU64>,
    done_rx: mpsc::Receiver<IncrementalDone>,
    temp_dir: PathBuf,
}

struct PendingIncremental {
    active: ActiveIncremental,
    worker: IncrementalWorker,
    done_tx: mpsc::Sender<IncrementalDone>,
}

struct IncrementalDone {
    session: RecordingSession,
    complete: bool,
    fallback_required: bool,
}

struct IncrementalWorker {
    app: AppHandle,
    engine: Arc<Engine>,
    cfg: AppConfig,
    audio_path: PathBuf,
    session_id: String,
    temp_dir: PathBuf,
    session_token: CancelToken,
    worker_cancel_token: CancelToken,
    stop_requested: Arc<AtomicBool>,
    final_duration_ms: Arc<AtomicU64>,
}

#[derive(Clone)]
struct SessionControl {
    id: String,
    cancel_token: CancelToken,
    temp_dir: Option<PathBuf>,
}

pub struct Engine {
    recording: Mutex<Option<ActiveRecording>>,
    active_session: Mutex<Option<SessionControl>>,
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
            active_session: Mutex::new(None),
            state: Mutex::new(StatePayload {
                stage: Stage::Idle,
                recording_started_ms: None,
                segments: Vec::new(),
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

    fn begin_session(&self, control: SessionControl) {
        *self.active_session.lock().unwrap() = Some(control);
    }

    fn is_session_current(&self, session_id: &str, token: &CancelToken) -> bool {
        if token.is_cancelled() {
            return false;
        }
        self.active_session
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| active.id == session_id && !active.cancel_token.is_cancelled())
    }

    fn set_state_for_session(
        &self,
        app: &AppHandle,
        session_id: &str,
        token: &CancelToken,
        update: impl FnOnce(&mut StatePayload),
    ) -> bool {
        let payload = {
            if token.is_cancelled() {
                return false;
            }
            let active = self.active_session.lock().unwrap();
            let Some(current) = active.as_ref() else {
                return false;
            };
            if current.id != session_id || current.cancel_token.is_cancelled() {
                return false;
            }
            let mut state = self.state.lock().unwrap();
            update(&mut state);
            state.clone()
        };
        let _ = app.emit(EVENT_STATE, &payload);
        crate::tray::sync(app, payload.stage);
        true
    }

    fn finish_session(
        &self,
        app: &AppHandle,
        session_id: &str,
        token: &CancelToken,
        update: impl FnOnce(&mut StatePayload),
    ) -> bool {
        let payload = {
            if token.is_cancelled() {
                return false;
            }
            let mut active = self.active_session.lock().unwrap();
            let Some(current) = active.as_ref() else {
                return false;
            };
            if current.id != session_id || current.cancel_token.is_cancelled() {
                return false;
            }
            let mut state = self.state.lock().unwrap();
            update(&mut state);
            let payload = state.clone();
            *active = None;
            payload
        };
        let _ = app.emit(EVENT_STATE, &payload);
        crate::tray::sync(app, payload.stage);
        true
    }

    fn cancel_current_session(&self) -> Option<SessionControl> {
        let session = self.active_session.lock().unwrap().take();
        if let Some(session) = session.as_ref() {
            session.cancel_token.cancel();
        }
        session
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
                s.segments.clear();
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
                    s.recording_started_ms = None;
                    s.segments.clear();
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
        let session_id = format!("{}-{}", now_ms(), std::process::id());
        let cancel_token = CancelToken::new();
        let pending_incremental = if cfg.incremental.enabled {
            let temp_dir = recorder::state_dir().join("incremental").join(&session_id);
            match self.prepare_incremental_worker(
                app,
                &cfg,
                audio_path.clone(),
                session_id.clone(),
                cancel_token.clone(),
                temp_dir,
            ) {
                Ok(active) => Some(active),
                Err(err) => {
                    self.set_state(app, |s| {
                        s.message = Some(format!(
                            "Incremental transcription unavailable; using final pass: {err:#}"
                        ));
                    });
                    None
                }
            }
        } else {
            None
        };
        let temp_dir = pending_incremental
            .as_ref()
            .map(|pending| pending.active.temp_dir.clone());
        self.begin_session(SessionControl {
            id: session_id.clone(),
            cancel_token: cancel_token.clone(),
            temp_dir,
        });
        let (incremental, pending_worker) = if let Some(PendingIncremental {
            active,
            worker,
            done_tx,
        }) = pending_incremental
        {
            (Some(active), Some((worker, done_tx)))
        } else {
            (None, None)
        };
        *self.recording.lock().unwrap() = Some(ActiveRecording {
            session_id,
            cancel_token,
            recording,
            incremental,
        });
        self.set_state(app, |s| {
            s.stage = Stage::Recording;
            s.recording_started_ms = Some(now_ms());
            s.segments.clear();
            s.error = None;
            if cfg.incremental.enabled && s.message.is_none() {
                s.message = Some("Incremental transcription active".into());
            } else if !cfg.incremental.enabled {
                s.message = None;
            }
        });

        if let Some((worker, done_tx)) = pending_worker {
            std::thread::spawn(move || {
                let done = run_incremental_worker(worker);
                let _ = done_tx.send(done);
            });
        }

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
        let Some(active) = self.recording.lock().unwrap().take() else {
            return;
        };
        self.levels_running.store(false, Ordering::SeqCst);
        let cfg = AppConfig::load();
        if cfg.general.sounds {
            sounds::play(sounds::Cue::Stop);
        }
        let _ = self.set_state_for_session(app, &active.session_id, &active.cancel_token, |s| {
            s.stage = Stage::Transcribing;
            s.recording_started_ms = None;
        });

        let engine = Arc::clone(self);
        let app = app.clone();
        std::thread::spawn(move || engine.run_pipeline(&app, cfg, active));
    }

    pub fn cancel(self: &Arc<Self>, app: &AppHandle) {
        self.levels_running.store(false, Ordering::SeqCst);
        let cfg = AppConfig::load();
        let active = self.recording.lock().unwrap().take();
        let cancelled_session = self.cancel_current_session();
        if let Some(session) = cancelled_session.as_ref().and_then(|s| s.temp_dir.as_ref()) {
            let _ = incremental::cleanup_session_dir(session, cfg.general.keep_audio);
        }
        if let Some(active) = active {
            active.cancel_token.cancel();
            if let Some(incremental) = active.incremental {
                incremental.worker_cancel_token.cancel();
                incremental.stop_requested.store(true, Ordering::SeqCst);
                if incremental
                    .done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .is_err()
                {
                    let _ = incremental::cleanup_session_dir(
                        &incremental.temp_dir,
                        cfg.general.keep_audio,
                    );
                }
            }
            active.recording.cancel();
        }
        self.set_state(app, |s| {
            s.stage = Stage::Idle;
            s.recording_started_ms = None;
            s.segments.clear();
            s.message = Some("Recording cancelled".into());
            s.error = None;
        });
    }

    fn prepare_incremental_worker(
        self: &Arc<Self>,
        app: &AppHandle,
        cfg: &AppConfig,
        audio_path: PathBuf,
        session_id: String,
        cancel_token: CancelToken,
        temp_dir: PathBuf,
    ) -> anyhow::Result<PendingIncremental> {
        fs::create_dir_all(&temp_dir)?;

        let worker_cancel_token = CancelToken::new();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let final_duration_ms = Arc::new(AtomicU64::new(0));
        let (done_tx, done_rx) = mpsc::channel();
        let worker = IncrementalWorker {
            app: app.clone(),
            engine: Arc::clone(self),
            cfg: cfg.clone(),
            audio_path,
            session_id,
            temp_dir: temp_dir.clone(),
            session_token: cancel_token,
            worker_cancel_token: worker_cancel_token.clone(),
            stop_requested: Arc::clone(&stop_requested),
            final_duration_ms: Arc::clone(&final_duration_ms),
        };

        let active = ActiveIncremental {
            worker_cancel_token,
            stop_requested,
            final_duration_ms,
            done_rx,
            temp_dir,
        };

        Ok(PendingIncremental {
            active,
            worker,
            done_tx,
        })
    }

    fn incremental_raw_text(
        &self,
        cfg: &AppConfig,
        duration_ms: u64,
        active: Option<ActiveIncremental>,
        session_id: &str,
        token: &CancelToken,
    ) -> Option<String> {
        let active = active?;
        active
            .final_duration_ms
            .store(duration_ms, Ordering::SeqCst);
        active.stop_requested.store(true, Ordering::SeqCst);

        for _ in 0..50 {
            if !self.is_session_current(session_id, token) {
                active.worker_cancel_token.cancel();
                let _ = incremental::cleanup_session_dir(&active.temp_dir, cfg.general.keep_audio);
                return None;
            }
            match active.done_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(done) if done.complete && !done.fallback_required => {
                    let raw = done.session.final_raw_text();
                    return (!raw.trim().is_empty()).then_some(raw);
                }
                Ok(_) => return None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }

        active.worker_cancel_token.cancel();
        let _ = incremental::cleanup_session_dir(&active.temp_dir, cfg.general.keep_audio);
        None
    }

    fn run_pipeline(self: Arc<Self>, app: &AppHandle, cfg: AppConfig, active: ActiveRecording) {
        let ActiveRecording {
            session_id,
            cancel_token,
            recording,
            incremental,
        } = active;

        let fail = |err: String| {
            if !self.is_session_current(&session_id, &cancel_token) {
                return;
            }
            if cfg.general.sounds {
                sounds::play(sounds::Cue::Error);
            }
            let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
                s.stage = Stage::Idle;
                s.recording_started_ms = None;
                s.segments.clear();
                s.error = Some(err);
                s.message = None;
            });
        };

        let (audio_path, duration_ms) = match recording.stop() {
            Ok(result) => result,
            Err(err) => return fail(format!("{err:#}")),
        };
        if !self.is_session_current(&session_id, &cancel_token) {
            if !cfg.general.keep_audio {
                let _ = fs::remove_file(&audio_path);
            }
            return;
        }

        let raw = match self.incremental_raw_text(
            &cfg,
            duration_ms,
            incremental,
            &session_id,
            &cancel_token,
        ) {
            Some(text) => Ok(text),
            None => {
                if !self.is_session_current(&session_id, &cancel_token) {
                    if !cfg.general.keep_audio {
                        let _ = fs::remove_file(&audio_path);
                    }
                    return;
                }
                stt::transcribe_with_cancel(&cfg.stt, &audio_path, || {
                    !self.is_session_current(&session_id, &cancel_token)
                })
            }
        };
        let raw = match raw {
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
        if !self.is_session_current(&session_id, &cancel_token) {
            return;
        }
        if raw.is_empty() {
            let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
                s.stage = Stage::Idle;
                s.recording_started_ms = None;
                s.segments.clear();
                s.message = Some("No speech detected".into());
                s.error = None;
            });
            return;
        }

        if !self.set_state_for_session(app, &session_id, &cancel_token, |s| {
            s.stage = Stage::Cleaning
        }) {
            return;
        }
        let outcome = cleanup::clean(&cfg, &raw);
        if !self.is_session_current(&session_id, &cancel_token) {
            return;
        }

        if !self.set_state_for_session(app, &session_id, &cancel_token, |s| {
            s.stage = Stage::Pasting
        }) {
            return;
        }
        let mut paste_cfg = cfg.paste.clone();
        if let Some(chord) = self.chord_override.lock().unwrap().clone() {
            paste_cfg.chord = chord;
        }
        let paste_error = paste::deliver(&paste_cfg, &outcome.text)
            .err()
            .map(|err| format!("paste failed (text copied if possible): {err:#}"));
        if !self.is_session_current(&session_id, &cancel_token) {
            return;
        }

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

        let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
            s.stage = Stage::Idle;
            s.recording_started_ms = None;
            s.segments.clear();
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

fn run_incremental_worker(worker: IncrementalWorker) -> IncrementalDone {
    let mut session = RecordingSession::new(worker.session_id.clone());
    let mut next_start_ms = 0u64;
    let mut segment_id = 0u64;
    let mut fallback_required = false;
    let mut complete = false;

    let target_ms = worker.cfg.incremental.target_ms.max(1_000);
    let max_ms = worker.cfg.incremental.max_ms.max(target_ms);
    let overlap_ms = worker.cfg.incremental.overlap_ms.min(target_ms / 2);
    let backlog_limit_ms = max_ms.saturating_mul(worker.cfg.incremental.max_queue.max(1) as u64);

    while !worker.worker_cancel_token.is_cancelled()
        && worker
            .engine
            .is_session_current(&worker.session_id, &worker.session_token)
    {
        let final_requested = worker.stop_requested.load(Ordering::SeqCst);
        let available_ms = available_audio_ms(&worker.audio_path, final_requested, &worker);
        if available_ms <= next_start_ms.saturating_add(250) {
            if final_requested {
                if available_ms > next_start_ms {
                    fallback_required = true;
                } else {
                    complete = true;
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }

        let buffered_ms = available_ms.saturating_sub(next_start_ms);
        if !final_requested && buffered_ms < target_ms {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        if !final_requested && buffered_ms > backlog_limit_ms {
            fallback_required = true;
        }
        if final_requested && buffered_ms > max_ms {
            fallback_required = true;
        }

        let desired_end_ms = if final_requested {
            next_start_ms.saturating_add(max_ms).min(available_ms)
        } else {
            next_start_ms.saturating_add(target_ms).min(available_ms)
        };
        let end_ms = choose_segment_end(
            &worker.audio_path,
            next_start_ms,
            desired_end_ms,
            available_ms,
            final_requested,
        );
        if end_ms <= next_start_ms {
            if final_requested {
                if available_ms > next_start_ms {
                    fallback_required = true;
                } else {
                    complete = true;
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }

        segment_id = segment_id.saturating_add(1);
        let slice_start_ms = next_start_ms.saturating_sub(overlap_ms);
        let segment_path = worker.temp_dir.join(format!("segment-{segment_id:04}.wav"));
        let slice = match audio_segments::slice_wav(
            &worker.audio_path,
            &segment_path,
            slice_start_ms,
            end_ms,
        ) {
            Ok(slice) if slice.sample_count > 0 => slice,
            Ok(_) if final_requested => {
                fallback_required = available_ms > next_start_ms;
                complete = !fallback_required;
                break;
            }
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            Err(err) if final_requested => {
                fallback_required = true;
                let segment = TranscriptSegment::failed(
                    segment_id,
                    slice_start_ms,
                    end_ms,
                    format!("{err:#}"),
                );
                session.upsert_segment(segment);
                emit_incremental_session(&worker, &session);
                break;
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
        };

        let transcribing = TranscriptSegment {
            id: segment_id,
            start_ms: slice.start_ms,
            end_ms: slice.end_ms,
            status: TranscriptSegmentStatus::Transcribing,
            raw_text: String::new(),
            cleaned_text: None,
            error: None,
        };
        session.upsert_segment(transcribing);
        emit_incremental_session(&worker, &session);

        let worker_token = worker.worker_cancel_token.clone();
        let session_token = worker.session_token.clone();
        let session_id = worker.session_id.clone();
        let engine = Arc::clone(&worker.engine);
        let result = stt::transcribe_with_cancel(&worker.cfg.stt, &segment_path, || {
            worker_token.is_cancelled() || !engine.is_session_current(&session_id, &session_token)
        });
        if !worker.cfg.general.keep_audio {
            let _ = fs::remove_file(&segment_path);
        }
        if worker.worker_cancel_token.is_cancelled()
            || !worker
                .engine
                .is_session_current(&worker.session_id, &worker.session_token)
        {
            break;
        }

        match result {
            Ok(text) => session.upsert_segment(TranscriptSegment::raw_ready(
                segment_id,
                slice.start_ms,
                slice.end_ms,
                text,
            )),
            Err(err) => {
                fallback_required = true;
                session.upsert_segment(TranscriptSegment::failed(
                    segment_id,
                    slice.start_ms,
                    slice.end_ms,
                    format!("{err:#}"),
                ));
            }
        }
        emit_incremental_session(&worker, &session);
        next_start_ms = slice.end_ms;

        if final_requested && next_start_ms >= available_ms {
            complete = true;
            break;
        }
    }

    if !worker.cfg.general.keep_audio {
        let _ = incremental::cleanup_session_dir(&worker.temp_dir, false);
    }

    IncrementalDone {
        session,
        complete: complete
            && !worker.worker_cancel_token.is_cancelled()
            && worker
                .engine
                .is_session_current(&worker.session_id, &worker.session_token)
            && !fallback_required,
        fallback_required,
    }
}

fn emit_incremental_session(worker: &IncrementalWorker, session: &RecordingSession) {
    let _ = worker.engine.set_state_for_session(
        &worker.app,
        &worker.session_id,
        &worker.session_token,
        |s| {
            s.segments = session.segments.clone();
        },
    );
}

fn available_audio_ms(path: &Path, final_requested: bool, worker: &IncrementalWorker) -> u64 {
    let file_ms = growing_audio_duration_ms(path).unwrap_or(0);
    if final_requested {
        file_ms.max(worker.final_duration_ms.load(Ordering::SeqCst))
    } else {
        file_ms
    }
}

fn growing_audio_duration_ms(path: &Path) -> anyhow::Result<u64> {
    let bytes = fs::metadata(path)?
        .len()
        .saturating_sub(audio_segments::WAV_HEADER_BYTES);
    let samples = bytes / audio_segments::BYTES_PER_SAMPLE;
    Ok(samples.saturating_mul(1_000) / audio_segments::SAMPLE_RATE_HZ as u64)
}

fn choose_segment_end(
    audio_path: &Path,
    next_start_ms: u64,
    desired_end_ms: u64,
    available_ms: u64,
    final_requested: bool,
) -> u64 {
    if final_requested {
        return available_ms;
    }

    let radius_ms = 500;
    let scan_start_ms = desired_end_ms.saturating_sub(radius_ms).max(next_start_ms);
    let scan_end_ms = desired_end_ms.saturating_add(radius_ms).min(available_ms);
    let Ok(samples) = audio_segments::read_samples(audio_path, scan_start_ms, scan_end_ms) else {
        return desired_end_ms.min(available_ms);
    };
    if samples.is_empty() {
        return desired_end_ms.min(available_ms);
    }

    let target_sample = ms_to_sample(desired_end_ms.saturating_sub(scan_start_ms));
    let radius_sample = ms_to_sample(radius_ms).min(samples.len());
    let boundary_sample =
        audio_segments::find_low_energy_boundary(&samples, target_sample, radius_sample);
    let boundary_ms = scan_start_ms.saturating_add(sample_to_ms(boundary_sample as u64));
    boundary_ms.clamp(next_start_ms.saturating_add(250), available_ms)
}

fn ms_to_sample(ms: u64) -> usize {
    (ms.saturating_mul(audio_segments::SAMPLE_RATE_HZ as u64) / 1_000) as usize
}

fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / audio_segments::SAMPLE_RATE_HZ as u64
}

pub fn engine(app: &AppHandle) -> Arc<Engine> {
    Arc::clone(&*app.state::<Arc<Engine>>())
}
