use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pickscribe::config::AppConfig;
use pickscribe::engine::{
    audio_segments, cleanup,
    incremental::{
        self, CancelToken, Control, IncrementalHost, RunResult, SchedulingConfig, SegmentJob,
    },
    levels::LevelMeter,
    paste, recorder,
    segments::{self, RecordingSession, TranscriptSegment, TranscriptSegmentStatus},
    sounds, stt,
};
use pickscribe::history::{HistoryDb, HistoryEntry, NewEntry};
use pickscribe::platform;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::file_job::FileJobControl;
use crate::lifecycle::{self, SessionSnapshot};

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

/// Explicit outcome of an incremental session at stop time.
enum IncrementalResult {
    /// Every segment finished; use the worker's assembled transcript as-is.
    Complete(String),
    /// The worker fell back, but a contiguous prefix of segments finished:
    /// only the remaining tail needs the final transcription pass.
    Partial(segments::SalvagedPrefix),
    /// Nothing usable — the final pass re-transcribes the whole recording.
    Unavailable,
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
}

struct SegmentCleanupWorker {
    cancel_token: CancelToken,
    jobs_tx: mpsc::SyncSender<TranscriptSegment>,
    results_rx: mpsc::Receiver<TranscriptSegment>,
}

impl Drop for SegmentCleanupWorker {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

#[derive(Clone)]
struct SessionControl {
    id: String,
    cancel_token: CancelToken,
    temp_dir: Option<PathBuf>,
}

impl SessionControl {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            id: self.id.clone(),
            cancelled: self.cancel_token.is_cancelled(),
        }
    }
}

pub struct Engine {
    recording: Mutex<Option<ActiveRecording>>,
    active_session: Mutex<Option<SessionControl>>,
    pub(crate) file_job: Mutex<Option<FileJobControl>>,
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
            file_job: Mutex::new(None),
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
        let active = self.active_session.lock().unwrap();
        let snapshot = active.as_ref().map(SessionControl::snapshot);
        lifecycle::session_is_current(snapshot.as_ref(), session_id, token.is_cancelled())
    }

    fn set_state_for_session(
        &self,
        app: &AppHandle,
        session_id: &str,
        token: &CancelToken,
        update: impl FnOnce(&mut StatePayload),
    ) -> bool {
        let payload = {
            let active = self.active_session.lock().unwrap();
            let snapshot = active.as_ref().map(SessionControl::snapshot);
            if !lifecycle::session_is_current(snapshot.as_ref(), session_id, token.is_cancelled()) {
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
            let mut active = self.active_session.lock().unwrap();
            let snapshot = active.as_ref().map(SessionControl::snapshot);
            if !lifecycle::session_is_current(snapshot.as_ref(), session_id, token.is_cancelled()) {
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
        match lifecycle::toggle_action(stage) {
            lifecycle::ToggleAction::Start => self.start(app),
            lifecycle::ToggleAction::Stop => self.stop(app),
            lifecycle::ToggleAction::Ignore => {}
        }
    }

    // TODO(#63): split legacy recording startup into capped helpers.
    #[allow(clippy::too_many_lines)]
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
            session_id: session_id.clone(),
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

        // Recorder warm-up check, off the command thread so toggling stays
        // responsive: if the recorder exited immediately, surface the error
        // and tear the session down.
        {
            let engine = Arc::clone(self);
            let check_app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                let err = {
                    let mut guard = engine.recording.lock().unwrap();
                    match guard.as_mut() {
                        Some(active) if active.session_id == session_id => {
                            active.recording.exit_error()
                        }
                        _ => None,
                    }
                };
                let Some(err) = err else {
                    return;
                };
                engine.cancel(&check_app);
                if AppConfig::load().general.sounds {
                    sounds::play(sounds::Cue::Error);
                }
                engine.set_state(&check_app, |s| {
                    s.stage = Stage::Idle;
                    s.recording_started_ms = None;
                    s.segments.clear();
                    s.error = Some(err);
                    s.message = None;
                });
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
            // Drain the recorder and incremental worker off the command
            // thread: recorder teardown and the worker drain can block for
            // seconds, and cancel must keep the UI responsive.
            let keep_audio = cfg.general.keep_audio;
            std::thread::spawn(move || {
                let ActiveRecording {
                    cancel_token,
                    recording,
                    incremental,
                    ..
                } = active;
                cancel_token.cancel();
                if let Some(incremental) = incremental {
                    incremental.worker_cancel_token.cancel();
                    incremental.stop_requested.store(true, Ordering::SeqCst);
                    recording.cancel();
                    if incremental
                        .done_rx
                        .recv_timeout(Duration::from_secs(2))
                        .is_err()
                    {
                        let _ = incremental::cleanup_session_dir(&incremental.temp_dir, keep_audio);
                    }
                } else {
                    recording.cancel();
                }
            });
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
        };

        let active = ActiveIncremental {
            worker_cancel_token,
            stop_requested,
            done_rx,
            temp_dir,
        };

        Ok(PendingIncremental {
            active,
            worker,
            done_tx,
        })
    }

    fn incremental_result(
        &self,
        cfg: &AppConfig,
        active: Option<ActiveIncremental>,
        session_id: &str,
        token: &CancelToken,
    ) -> IncrementalResult {
        let Some(active) = active else {
            return IncrementalResult::Unavailable;
        };
        active.stop_requested.store(true, Ordering::SeqCst);

        for _ in 0..50 {
            if !self.is_session_current(session_id, token) {
                active.worker_cancel_token.cancel();
                let _ = incremental::cleanup_session_dir(&active.temp_dir, cfg.general.keep_audio);
                return IncrementalResult::Unavailable;
            }
            match active.done_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(done) if done.complete && !done.fallback_required => {
                    let raw = done.session.final_raw_text();
                    return if raw.trim().is_empty() {
                        IncrementalResult::Unavailable
                    } else {
                        IncrementalResult::Complete(raw)
                    };
                }
                Ok(done) => {
                    // Fallback: preserve the finished contiguous prefix when
                    // valid so the final pass only re-transcribes the tail
                    // instead of silently discarding all incremental work.
                    return match segments::salvage_completed_prefix(&done.session.segments) {
                        Some(prefix) if !prefix.raw_text.trim().is_empty() => {
                            IncrementalResult::Partial(prefix)
                        }
                        _ => IncrementalResult::Unavailable,
                    };
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return IncrementalResult::Unavailable,
            }
        }

        // Drain cap (5s) lapsed: the worker is still busy — typically on a
        // slow final segment. Salvage the finished prefix from the live
        // state (the worker publishes segments there as they complete)
        // instead of silently discarding all incremental work.
        active.worker_cancel_token.cancel();
        let _ = incremental::cleanup_session_dir(&active.temp_dir, cfg.general.keep_audio);
        let live_segments = self.state.lock().unwrap().segments.clone();
        match segments::salvage_completed_prefix(&live_segments) {
            Some(prefix) if !prefix.raw_text.trim().is_empty() => {
                IncrementalResult::Partial(prefix)
            }
            _ => IncrementalResult::Unavailable,
        }
    }

    // TODO(#63): split legacy pipeline orchestration into capped helpers.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
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
            let (error, message) = lifecycle::failure_outcome(err);
            let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
                s.stage = Stage::Idle;
                s.recording_started_ms = None;
                s.segments.clear();
                s.error = error;
                s.message = message;
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

        let raw = match self.incremental_result(&cfg, incremental, &session_id, &cancel_token) {
            IncrementalResult::Complete(text) => Ok(text),
            outcome => {
                if !self.is_session_current(&session_id, &cancel_token) {
                    if !cfg.general.keep_audio {
                        let _ = fs::remove_file(&audio_path);
                    }
                    return;
                }
                let is_cancelled = || !self.is_session_current(&session_id, &cancel_token);
                match outcome {
                    IncrementalResult::Partial(prefix) => {
                        match slice_fallback_tail(&cfg, &audio_path, &prefix) {
                            // Recording ended inside the already-transcribed
                            // prefix — nothing left to transcribe.
                            Ok(None) => Ok(prefix.raw_text),
                            Ok(Some(tail_path)) => {
                                let tail =
                                    stt::transcribe_with_cancel(&cfg.stt, &tail_path, is_cancelled);
                                let _ = fs::remove_file(&tail_path);
                                tail.map(|tail| {
                                    segments::merge_texts([prefix.raw_text.as_str(), tail.as_str()])
                                })
                            }
                            // Tail slicing failed — fall back to the whole
                            // recording rather than erroring the pipeline.
                            Err(_) => {
                                stt::transcribe_with_cancel(&cfg.stt, &audio_path, is_cancelled)
                            }
                        }
                    }
                    _ => stt::transcribe_with_cancel(&cfg.stt, &audio_path, is_cancelled),
                }
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
            let (error, message) = lifecycle::no_speech_outcome();
            let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
                s.stage = Stage::Idle;
                s.recording_started_ms = None;
                s.segments.clear();
                s.message = message;
                s.error = error;
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
        let delivery_cfg = paste::DeliveryConfig::from(&paste_cfg);
        let paste_error = paste::deliver(&delivery_cfg, &outcome.text)
            .into_result()
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
            source_file: None,
            segments_json: None,
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
                    source_file: entry.source_file,
                    segments_json: entry.segments_json,
                    word_count: pickscribe::history::count_words(&outcome.text),
                })
            }
            Err(_) => None,
        };
        let delivered = lifecycle::delivery_outcome(paste_error, &outcome, last_entry);

        let _ = self.finish_session(app, &session_id, &cancel_token, |s| {
            s.stage = Stage::Idle;
            s.recording_started_ms = None;
            s.segments.clear();
            s.error = delivered.error;
            s.message = Some(delivered.message);
            s.last_entry = delivered.last_entry;
        });
    }
}

/// Explicit fallback policy: when the incremental worker cannot finish but a
/// contiguous prefix of segments completed, slice out only the remaining
/// tail (with the configured overlap) so the final pass avoids
/// re-transcribing work that already succeeded. Returns `Ok(None)` when the
/// recording ends inside the transcribed prefix.
fn slice_fallback_tail(
    cfg: &AppConfig,
    audio_path: &Path,
    prefix: &segments::SalvagedPrefix,
) -> anyhow::Result<Option<PathBuf>> {
    let target_ms = cfg.incremental.target_ms.max(1_000);
    let overlap_ms = cfg.incremental.overlap_ms.min(target_ms / 2);
    let available_ms = growing_audio_duration_ms(audio_path)?;
    if available_ms <= prefix.resume_from_ms {
        return Ok(None);
    }
    let start_ms = prefix.resume_from_ms.saturating_sub(overlap_ms);
    let tail_path = audio_path.with_extension("tail.wav");
    let slice = audio_segments::slice_wav(audio_path, &tail_path, start_ms, available_ms)?;
    if slice.sample_count == 0 {
        let _ = fs::remove_file(&tail_path);
        return Ok(None);
    }
    Ok(Some(tail_path))
}

/// The desktop worker's [`IncrementalHost`]: an in-process background
/// thread whose stop/cancel signals are a session-scoped [`CancelToken`]
/// pair (the worker's own, plus the recording session's) instead of the
/// CLI's files. There is no orphan/parent-death case here — the owning
/// process is this one — so `control()` never returns `Abandoned`.
struct TauriIncrementalHost {
    worker: IncrementalWorker,
    cleanup_worker: Option<SegmentCleanupWorker>,
}

impl IncrementalHost for TauriIncrementalHost {
    fn control(&self) -> Control {
        if self.worker.worker_cancel_token.is_cancelled()
            || !self
                .worker
                .engine
                .is_session_current(&self.worker.session_id, &self.worker.session_token)
        {
            return Control::Cancelled;
        }
        if self.worker.stop_requested.load(Ordering::SeqCst) {
            return Control::Stopping;
        }
        Control::Continue
    }

    fn keep_audio(&self) -> bool {
        self.worker.cfg.general.keep_audio
    }

    fn segment_path(&self, segment_id: u64) -> PathBuf {
        self.worker
            .temp_dir
            .join(format!("segment-{segment_id:04}.wav"))
    }

    fn transcribe(&mut self, job: &SegmentJob) -> anyhow::Result<String> {
        stt::transcribe_with_cancel(&self.worker.cfg.stt, &job.audio_path, || {
            matches!(self.control(), Control::Cancelled)
        })
    }

    fn try_queue_cleanup(&mut self, segment: TranscriptSegment) -> bool {
        let Some(cleanup_worker) = self.cleanup_worker.as_ref() else {
            return false;
        };
        cleanup_worker.jobs_tx.try_send(segment).is_ok()
    }

    fn drain_cleanup(&mut self) -> Vec<TranscriptSegment> {
        let Some(cleanup_worker) = self.cleanup_worker.as_ref() else {
            return Vec::new();
        };
        let mut drained = Vec::new();
        while let Ok(segment) = cleanup_worker.results_rx.try_recv() {
            drained.push(segment);
        }
        drained
    }

    fn publish(&mut self, session: &RecordingSession) {
        emit_incremental_session(&self.worker, session);
    }

    fn cleanup_artifacts(&mut self) {
        // Unreachable in practice: this host never returns Control::Abandoned.
        if !self.worker.cfg.general.keep_audio {
            let _ = incremental::cleanup_session_dir(&self.worker.temp_dir, false);
        }
    }
}

fn run_incremental_worker(worker: IncrementalWorker) -> IncrementalDone {
    let scheduling_cfg = SchedulingConfig::new(
        worker.cfg.incremental.target_ms,
        worker.cfg.incremental.max_ms,
        worker.cfg.incremental.overlap_ms,
        worker.cfg.incremental.max_queue,
    );
    let cleanup_worker = worker
        .cfg
        .incremental
        .cleanup_segments
        .then(|| start_segment_cleanup_worker(&worker));
    let audio_path = worker.audio_path.clone();
    let session_id = worker.session_id.clone();
    let keep_audio = worker.cfg.general.keep_audio;
    let temp_dir = worker.temp_dir.clone();

    let mut host = TauriIncrementalHost {
        worker,
        cleanup_worker,
    };
    let result = incremental::run(&mut host, &audio_path, session_id, scheduling_cfg);

    if !keep_audio {
        let _ = incremental::cleanup_session_dir(&temp_dir, false);
    }

    match result {
        RunResult::Abandoned => IncrementalDone {
            session: RecordingSession::new(host.worker.session_id.clone()),
            complete: false,
            fallback_required: false,
        },
        RunResult::Finished(outcome) => {
            // Defensive re-check: cancellation/supersession racing in right
            // as the last segment finished must not report `complete`.
            let complete = outcome.complete
                && !host.worker.worker_cancel_token.is_cancelled()
                && host
                    .worker
                    .engine
                    .is_session_current(&host.worker.session_id, &host.worker.session_token);
            IncrementalDone {
                session: outcome.session,
                complete: complete && !outcome.fallback_required,
                fallback_required: outcome.fallback_required,
            }
        }
    }
}

fn start_segment_cleanup_worker(worker: &IncrementalWorker) -> SegmentCleanupWorker {
    let queue_size = worker.cfg.incremental.max_queue.max(1);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<TranscriptSegment>(queue_size);
    let (results_tx, results_rx) = mpsc::channel::<TranscriptSegment>();
    let cleanup_cancel_token = CancelToken::new();
    let cfg = worker.cfg.clone();
    let engine = Arc::clone(&worker.engine);
    let session_id = worker.session_id.clone();
    let session_token = worker.session_token.clone();
    let worker_cancel_token = worker.worker_cancel_token.clone();
    let thread_cancel_token = cleanup_cancel_token.clone();

    std::thread::spawn(move || {
        while let Ok(raw) = jobs_rx.recv() {
            if thread_cancel_token.is_cancelled()
                || worker_cancel_token.is_cancelled()
                || !engine.is_session_current(&session_id, &session_token)
            {
                break;
            }

            let outcome = cleanup::clean_segment(&cfg, &raw.raw_text);
            if thread_cancel_token.is_cancelled()
                || worker_cancel_token.is_cancelled()
                || !engine.is_session_current(&session_id, &session_token)
            {
                break;
            }

            let segment = if outcome.cleaned {
                TranscriptSegment {
                    status: TranscriptSegmentStatus::Cleaned,
                    cleaned_text: Some(outcome.text),
                    ..raw
                }
            } else {
                raw
            };
            if results_tx.send(segment).is_err() {
                break;
            }
        }
    });

    SegmentCleanupWorker {
        cancel_token: cleanup_cancel_token,
        jobs_tx,
        results_rx,
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

fn growing_audio_duration_ms(path: &Path) -> anyhow::Result<u64> {
    let bytes = fs::metadata(path)?
        .len()
        .saturating_sub(audio_segments::WAV_HEADER_BYTES);
    let samples = bytes / audio_segments::BYTES_PER_SAMPLE;
    Ok(samples.saturating_mul(1_000) / audio_segments::SAMPLE_RATE_HZ as u64)
}

pub fn engine(app: &AppHandle) -> Arc<Engine> {
    Arc::clone(&*app.state::<Arc<Engine>>())
}

// Transition validity, session-current gating, and terminal-outcome
// projection are characterized in `lifecycle.rs`, which this module calls
// into. See that module's tests for duplicate start/toggle, stop before
// init finishes, cancellation at every pipeline checkpoint, stale
// completion after cancel/new session, cleanup failure with raw delivery,
// and history failure.
