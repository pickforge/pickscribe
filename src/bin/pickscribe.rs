use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use pickscribe::{
    config::{AppConfig, IncrementalConfig},
    engine::{
        audio_segments, cleanup as cleanup_engine,
        segments::{RecordingSession, TranscriptSegment, TranscriptSegmentStatus},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{self as unix_fs, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "pickscribe",
    about = "PickScribe by Pickforge Studio: toggle record -> local Whisper STT -> AI cleanup -> paste"
)]
struct Args {
    /// Action to run. toggle starts recording if idle, stops/transcribes if recording.
    #[arg(value_enum, default_value = "toggle")]
    action: Action,

    /// Directory for runtime state and temporary recordings.
    #[arg(long, env = "PICKSCRIBE_STATE_DIR")]
    state_dir: Option<PathBuf>,

    /// Audio recorder command. The MVP expects pw-record-compatible flags.
    #[arg(long, default_value = "pw-record", env = "PICKSCRIBE_RECORDER")]
    recorder: String,

    /// PipeWire target node/name for the microphone. Leave empty for default input.
    #[arg(long, env = "PICKSCRIBE_AUDIO_TARGET")]
    audio_target: Option<String>,

    /// Whisper model path. If omitted, common Arch whisper.cpp model paths/wrappers are auto-detected.
    #[arg(long, env = "PICKSCRIBE_WHISPER_MODEL")]
    whisper_model: Option<PathBuf>,

    /// Whisper language, e.g. auto, en, pt. Use auto with a multilingual model for English + Brazilian Portuguese.
    #[arg(long, default_value = "auto", env = "PICKSCRIBE_LANGUAGE")]
    language: Option<String>,

    /// Custom STT shell command. Supports placeholders: {audio}, {model}, {output}.
    #[arg(long, env = "PICKSCRIBE_STT_COMMAND")]
    stt_command: Option<String>,

    /// Cleanup command to run after transcription.
    #[arg(long, env = "PICKSCRIBE_CLEANUP_COMMAND")]
    cleanup_command: Option<String>,

    /// Disable LLM cleanup, but still copy/type via pickscribe-cleanup.
    #[arg(long)]
    no_llm: bool,

    /// Do not copy final text to clipboard.
    #[arg(long)]
    no_copy: bool,

    /// Do not type final text into the active window.
    #[arg(long)]
    no_paste: bool,

    /// Print final cleaned text.
    #[arg(long)]
    print: bool,

    /// Print final cleaned text only; implies no copy and no paste.
    #[arg(long)]
    stdout_only: bool,

    /// Keep recorded WAV and transcript files instead of deleting them after stop.
    #[arg(long, env = "PICKSCRIBE_KEEP_AUDIO")]
    keep_audio: bool,

    /// Local whisper.cpp git checkout used by update-whisper.
    #[arg(long, env = "PICKSCRIBE_WHISPER_CPP_SRC")]
    whisper_cpp_src: Option<PathBuf>,

    /// Directory where update-whisper stores downloaded GGML models.
    #[arg(long, env = "PICKSCRIBE_WHISPER_MODEL_DIR")]
    whisper_model_dir: Option<PathBuf>,

    /// GGML model name downloaded by update-whisper, e.g. base, small, large-v3-turbo.
    #[arg(long, default_value = "base", env = "PICKSCRIBE_WHISPER_MODEL_NAME")]
    whisper_model_name: String,

    /// Periodic whisper.cpp update behavior when starting recording.
    #[arg(
        long,
        value_enum,
        default_value = "off",
        env = "PICKSCRIBE_AUTO_UPDATE_WHISPER"
    )]
    auto_update_whisper: AutoUpdateWhisper,

    /// Minimum hours between automatic whisper.cpp update checks. Use 0 to check every start.
    #[arg(long, default_value_t = 168, env = "PICKSCRIBE_UPDATE_INTERVAL_HOURS")]
    update_interval_hours: u64,

    /// Disable desktop notifications.
    #[arg(long)]
    no_notify: bool,

    /// Transcribe finalized chunks while recording; final output still uses one cleanup pass.
    #[arg(long)]
    incremental: bool,

    /// Clean finalized incremental chunks while recording. Runs cleanup in stdout-only mode.
    #[arg(long)]
    incremental_cleanup: bool,

    #[arg(long, hide = true)]
    run_incremental_worker: bool,

    #[arg(long, hide = true)]
    incremental_worker_state: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum Action {
    Toggle,
    Start,
    Stop,
    Status,
    Cancel,
    /// Check whether the local whisper.cpp checkout is behind upstream.
    CheckWhisper,
    /// Pull, rebuild, relink whisper-cli, and ensure the configured model exists.
    UpdateWhisper,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum AutoUpdateWhisper {
    /// Never check automatically.
    Off,
    /// Check periodically and notify when an update is available.
    Check,
    /// Check periodically and automatically pull/rebuild when an update is available.
    Install,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecordingState {
    pid: u32,
    audio_path: PathBuf,
    log_path: PathBuf,
    started_unix_secs: u64,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    incremental: Option<IncrementalWorkerState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IncrementalWorkerState {
    pid: u32,
    #[serde(default)]
    worker_started_ticks: Option<u64>,
    session_id: String,
    temp_dir: PathBuf,
    output_path: PathBuf,
    stop_path: PathBuf,
    cancel_path: PathBuf,
    log_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IncrementalWorkerOutput {
    session: RecordingSession,
    complete: bool,
    fallback_required: bool,
    error: Option<String>,
    updated_unix_secs: u64,
}

struct SegmentCleanupWorker {
    cancel: Arc<AtomicBool>,
    jobs_tx: Option<mpsc::SyncSender<TranscriptSegment>>,
    results_rx: mpsc::Receiver<TranscriptSegment>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for SegmentCleanupWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.jobs_tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.run_incremental_worker {
        return run_incremental_worker_command(&args);
    }

    match args.action {
        Action::Toggle => toggle(&args),
        Action::Start => start_recording(&args),
        Action::Stop => stop_recording(&args, false),
        Action::Status => print_status(&args),
        Action::Cancel => stop_recording(&args, true),
        Action::CheckWhisper => check_whisper(&args),
        Action::UpdateWhisper => update_whisper(&args),
    }
}

fn toggle(args: &Args) -> Result<()> {
    if let Some(state) = read_active_state(args)? {
        println!("Stopping recording from pid {}...", state.pid);
        stop_recording(args, false)
    } else {
        start_recording(args)
    }
}

fn start_recording(args: &Args) -> Result<()> {
    if let Some(state) = read_active_state(args)? {
        notify(args, "PickScribe", "Already recording");
        println!("Already recording with pid {}", state.pid);
        return Ok(());
    }

    if let Err(err) = maybe_auto_update_whisper(args) {
        notify(
            args,
            "PickScribe",
            "Whisper update check failed; continuing recording.",
        );
        eprintln!("warning: whisper update check failed: {err:#}");
    }

    let dir = state_dir(args)?;
    prepare_state_dir(args, &dir)?;

    let stamp = unix_secs();
    let audio_path = dir.join(format!("recording-{stamp}.wav"));
    let log_path = dir.join(format!("recording-{stamp}.log"));
    let log = File::create(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;

    let mut cmd = Command::new(&args.recorder);
    cmd.arg("--media-category")
        .arg("Capture")
        .arg("--media-role")
        .arg("Communication")
        .arg("--rate")
        .arg("16000")
        .arg("--channels")
        .arg("1")
        .arg("--format")
        .arg("s16");

    if let Some(target) = args
        .audio_target
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        cmd.arg("--target").arg(target);
    }

    let child = cmd
        .arg(&audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .with_context(|| format!("failed to start recorder command `{}`", args.recorder))?;

    let mut state = RecordingState {
        pid: child.id(),
        audio_path,
        log_path,
        started_unix_secs: stamp,
        session_id: None,
        incremental: None,
    };

    thread::sleep(Duration::from_millis(250));
    if !pid_alive(state.pid) {
        let log = fs::read_to_string(&state.log_path).unwrap_or_default();
        bail!("recorder exited immediately. Log:\n{log}");
    }

    if incremental_enabled(args) {
        match start_incremental_worker(args, &dir, &mut state) {
            Ok(()) => {}
            Err(err) => {
                notify(
                    args,
                    "PickScribe",
                    "Incremental transcription unavailable; using final pass.",
                );
                eprintln!("warning: incremental transcription unavailable: {err:#}");
                state.session_id = None;
                state.incremental = None;
            }
        }
    }

    if let Err(err) = write_state(args, &state) {
        signal_incremental_cancel(state.incremental.as_ref());
        stop_recorder(state.pid)?;
        terminate_incremental_worker(state.incremental.as_ref());
        cleanup_files(args, &state, None);
        cleanup_incremental_files(args, state.incremental.as_ref());
        return Err(err);
    }
    notify(
        args,
        "PickScribe",
        "Recording started. Run pickscribe again to stop.",
    );
    println!("Recording started. Run `pickscribe` again to stop.");
    Ok(())
}

fn stop_recording(args: &Args, cancel: bool) -> Result<()> {
    let state_path = state_path(args)?;
    let Some(state) = read_state_file(&state_path)? else {
        notify(args, "PickScribe", "Not recording");
        println!("Not recording.");
        return Ok(());
    };

    if cancel {
        signal_incremental_cancel(state.incremental.as_ref());
        stop_recorder(state.pid)?;
        terminate_incremental_worker(state.incremental.as_ref());
        let _ = fs::remove_file(&state_path);
        notify(args, "PickScribe", "Recording cancelled");
        cleanup_files(args, &state, None);
        cleanup_incremental_files(args, state.incremental.as_ref());
        println!("Recording cancelled.");
        return Ok(());
    }

    signal_incremental_stopping(state.incremental.as_ref());
    if let Err(err) = stop_recorder(state.pid) {
        signal_incremental_cancel(state.incremental.as_ref());
        terminate_incremental_worker(state.incremental.as_ref());
        return Err(err);
    }
    signal_incremental_stop(state.incremental.as_ref());
    let _ = fs::remove_file(&state_path);

    let result = finish_recording(args, &state);
    if result.is_err() {
        signal_incremental_cancel(state.incremental.as_ref());
        terminate_incremental_worker(state.incremental.as_ref());
    }
    cleanup_incremental_files(args, state.incremental.as_ref());
    result
}

fn finish_recording(args: &Args, state: &RecordingState) -> Result<()> {
    if !state.audio_path.exists() {
        bail!(
            "recording file does not exist: {}",
            state.audio_path.display()
        );
    }

    let size = fs::metadata(&state.audio_path)
        .with_context(|| format!("failed to stat {}", state.audio_path.display()))?
        .len();

    if size < 8_000 {
        bail!(
            "recording looks too small ({} bytes): {}",
            size,
            state.audio_path.display()
        );
    }

    notify(args, "PickScribe", "Transcribing...");
    println!("Transcribing {}...", state.audio_path.display());

    let transcript_path = transcript_txt_path_for(&state.audio_path);
    let transcript = incremental_transcript(state.incremental.as_ref())
        .map(Ok)
        .unwrap_or_else(|| transcribe(args, &state.audio_path))?;
    let transcript = cleanup_transcript(&transcript);

    if transcript.trim().is_empty() {
        notify(args, "PickScribe", "No speech detected");
        cleanup_files(args, state, Some(&transcript_path));
        cleanup_incremental_files(args, state.incremental.as_ref());
        println!("No speech detected.");
        return Ok(());
    }

    notify(args, "PickScribe", "Cleaning and pasting...");
    println!("Cleaning and pasting...");
    run_cleanup(args, &transcript)?;
    notify(args, "PickScribe", "Done");

    cleanup_files(args, state, Some(&transcript_path));
    Ok(())
}

fn print_status(args: &Args) -> Result<()> {
    match read_active_state(args)? {
        Some(state) => {
            println!("recording");
            println!("pid: {}", state.pid);
            println!("audio: {}", state.audio_path.display());
            println!("started: {}", state.started_unix_secs);
            if let Some(incremental) = state.incremental.as_ref() {
                println!("incremental: true");
                println!("incremental_worker_pid: {}", incremental.pid);
                if let Ok(Some(output)) = read_incremental_output(&incremental.output_path) {
                    println!("incremental_segments: {}", output.session.segments.len());
                    println!("incremental_complete: {}", output.complete);
                    println!("incremental_fallback: {}", output.fallback_required);
                }
            }
        }
        None => println!("idle"),
    }
    Ok(())
}

fn incremental_enabled(args: &Args) -> bool {
    args.incremental || env_flag_enabled("PICKSCRIBE_INCREMENTAL_DICTATION")
}

fn env_flag_enabled(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn incremental_config() -> IncrementalConfig {
    AppConfig::load().incremental
}

fn start_incremental_worker(args: &Args, dir: &Path, state: &mut RecordingState) -> Result<()> {
    let session_id = format!("cli-{}-{}", state.started_unix_secs, state.pid);
    let temp_dir = dir.join("incremental").join(&session_id);
    ensure_private_dir(&temp_dir)?;

    let worker_state = IncrementalWorkerState {
        pid: 0,
        worker_started_ticks: None,
        session_id: session_id.clone(),
        temp_dir: temp_dir.clone(),
        output_path: temp_dir.join("worker-output.json"),
        stop_path: temp_dir.join("stop"),
        cancel_path: temp_dir.join("cancel"),
        log_path: temp_dir.join("worker.log"),
    };
    let worker_state_path = temp_dir.join("worker-state.json");

    state.session_id = Some(session_id);
    state.incremental = Some(worker_state.clone());
    write_state_file(&worker_state_path, state)?;

    let log = File::create(&worker_state.log_path)
        .with_context(|| format!("failed to create {}", worker_state.log_path.display()))?;
    let mut cmd = Command::new(env::current_exe().context("failed to locate pickscribe binary")?);
    cmd.arg("--run-incremental-worker")
        .arg("--state-dir")
        .arg(dir)
        .arg("--incremental-worker-state")
        .arg(&worker_state_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    append_worker_args(&mut cmd, args);

    let mut child = cmd
        .spawn()
        .context("failed to start incremental worker process")?;
    let worker_started_ticks = match process_start_ticks(child.id()) {
        Some(ticks) => ticks,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            bail!("failed to read incremental worker process identity");
        }
    };
    if let Some(incremental) = state.incremental.as_mut() {
        incremental.pid = child.id();
        incremental.worker_started_ticks = Some(worker_started_ticks);
    }

    Ok(())
}

fn append_worker_args(cmd: &mut Command, args: &Args) {
    if let Some(model) = args.whisper_model.as_ref() {
        cmd.arg("--whisper-model").arg(model);
    }
    if let Some(language) = args.language.as_deref() {
        cmd.arg("--language").arg(language);
    }
    if let Some(stt_command) = args.stt_command.as_deref() {
        cmd.arg("--stt-command").arg(stt_command);
    }
    if let Some(cleanup_command) = args.cleanup_command.as_deref() {
        cmd.arg("--cleanup-command").arg(cleanup_command);
    }
    if args.no_llm {
        cmd.arg("--no-llm");
    }
    if args.stdout_only {
        cmd.arg("--stdout-only");
    }
    if args.no_paste {
        cmd.arg("--no-paste");
    }
    if args.incremental_cleanup {
        cmd.arg("--incremental-cleanup");
    }
    if args.keep_audio {
        cmd.arg("--keep-audio");
    }
}

fn run_incremental_worker_command(args: &Args) -> Result<()> {
    let worker_state_path = match args.incremental_worker_state.as_ref() {
        Some(path) => path.clone(),
        None => state_path(args)?,
    };
    let state = read_state_file(&worker_state_path)?
        .context("incremental worker started without recording state")?;
    let Some(worker) = state.incremental.clone() else {
        bail!("incremental worker started without worker state");
    };

    if let Err(err) = run_incremental_worker_loop(args, &state, &worker) {
        let mut session = RecordingSession::new(worker.session_id.clone());
        write_incremental_output(
            &worker.output_path,
            &mut session,
            false,
            true,
            Some(format!("{err:#}")),
        )?;
    }
    Ok(())
}

fn run_incremental_worker_loop(
    args: &Args,
    state: &RecordingState,
    worker: &IncrementalWorkerState,
) -> Result<()> {
    let mut session = RecordingSession::new(worker.session_id.clone());
    let mut next_start_ms = 0u64;
    let mut segment_id = 0u64;
    let mut fallback_required = false;
    let mut complete = false;

    let cfg = incremental_config();
    let target_ms = cfg.target_ms.max(1_000);
    let max_ms = cfg.max_ms.max(target_ms);
    let overlap_ms = cfg.overlap_ms.min(target_ms / 2);
    let backlog_limit_ms = max_ms.saturating_mul(cfg.max_queue.max(1) as u64);
    let state_file = state_path(args)?;
    let stopping_path = incremental_stopping_path(worker);
    let startup_deadline = Instant::now() + Duration::from_secs(30);
    let mut saw_state_file = state_file.exists();
    let local_only = AppConfig::load().general.local_only;
    let cleanup_worker = if incremental_segment_cleanup_enabled(args) {
        Some(start_segment_cleanup_worker(
            args,
            worker,
            &state_file,
            &stopping_path,
            state.pid,
            local_only,
        ))
    } else {
        None
    };

    write_incremental_output(&worker.output_path, &mut session, false, false, None)?;

    while !worker.cancel_path.exists() {
        drain_segment_cleanup_results(
            worker,
            &mut session,
            cleanup_worker.as_ref(),
            fallback_required,
        )?;

        let final_requested = worker.stop_path.exists();
        let stopping_requested = stopping_path.exists();
        if state_file.exists() {
            saw_state_file = true;
        }
        if !final_requested
            && !stopping_requested
            && (saw_state_file || Instant::now() >= startup_deadline)
            && (!state_file.exists() || !pid_alive(state.pid))
        {
            cleanup_incremental_files(args, Some(worker));
            return Ok(());
        }
        let available_ms = growing_audio_duration_ms(&state.audio_path).unwrap_or(0);
        if available_ms <= next_start_ms.saturating_add(250) {
            if final_requested {
                if available_ms > next_start_ms {
                    fallback_required = true;
                } else {
                    complete = true;
                }
                break;
            }
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        let buffered_ms = available_ms.saturating_sub(next_start_ms);
        if !final_requested && buffered_ms < target_ms {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        if !final_requested && buffered_ms > backlog_limit_ms {
            fallback_required = true;
            write_incremental_output(&worker.output_path, &mut session, false, true, None)?;
            break;
        }
        if final_requested && buffered_ms > max_ms {
            fallback_required = true;
            write_incremental_output(&worker.output_path, &mut session, false, true, None)?;
            break;
        }

        let desired_end_ms = if final_requested {
            next_start_ms.saturating_add(max_ms).min(available_ms)
        } else {
            next_start_ms.saturating_add(target_ms).min(available_ms)
        };
        let end_ms = choose_segment_end(
            &state.audio_path,
            next_start_ms,
            desired_end_ms,
            available_ms,
            final_requested,
        );
        if end_ms <= next_start_ms {
            if final_requested {
                fallback_required = available_ms > next_start_ms;
                complete = !fallback_required;
                break;
            }
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        segment_id = segment_id.saturating_add(1);
        let slice_start_ms = next_start_ms.saturating_sub(overlap_ms);
        let segment_path = worker.temp_dir.join(format!("segment-{segment_id:04}.wav"));
        let slice = match audio_segments::slice_wav(
            &state.audio_path,
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
                thread::sleep(Duration::from_millis(250));
                continue;
            }
            Err(err) if final_requested => {
                fallback_required = true;
                session.upsert_segment(TranscriptSegment::failed(
                    segment_id,
                    slice_start_ms,
                    end_ms,
                    format!("{err:#}"),
                ));
                write_incremental_output(&worker.output_path, &mut session, false, true, None)?;
                break;
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(250));
                continue;
            }
        };

        session.upsert_segment(TranscriptSegment {
            id: segment_id,
            start_ms: slice.start_ms,
            end_ms: slice.end_ms,
            status: TranscriptSegmentStatus::Transcribing,
            raw_text: String::new(),
            cleaned_text: None,
            error: None,
        });
        write_incremental_output(&worker.output_path, &mut session, false, false, None)?;

        let result = transcribe_incremental_segment(args, &segment_path, || {
            worker.cancel_path.exists()
                || (!worker.stop_path.exists()
                    && !stopping_path.exists()
                    && (saw_state_file || Instant::now() >= startup_deadline)
                    && (!state_file.exists() || !pid_alive(state.pid)))
        })
        .map(|text| cleanup_transcript(&text));
        if !args.keep_audio {
            let _ = fs::remove_file(&segment_path);
        }
        if worker.cancel_path.exists() {
            break;
        }

        match result {
            Ok(text) => {
                let raw = TranscriptSegment::raw_ready(
                    segment_id,
                    slice.start_ms,
                    slice.end_ms,
                    text.clone(),
                );
                session.upsert_segment(raw.clone());
                write_incremental_output(
                    &worker.output_path,
                    &mut session,
                    false,
                    fallback_required,
                    None,
                )?;

                queue_segment_cleanup(
                    worker,
                    &mut session,
                    cleanup_worker.as_ref(),
                    raw,
                    fallback_required,
                )?;
            }
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
        write_incremental_output(
            &worker.output_path,
            &mut session,
            false,
            fallback_required,
            None,
        )?;
        if fallback_required {
            break;
        }

        next_start_ms = slice.end_ms;
        if final_requested && next_start_ms >= available_ms {
            complete = true;
            break;
        }
    }

    drain_segment_cleanup_results(
        worker,
        &mut session,
        cleanup_worker.as_ref(),
        fallback_required,
    )?;
    stop_segment_cleanup_worker(cleanup_worker);

    if worker.cancel_path.exists() {
        return Ok(());
    }

    write_incremental_output(
        &worker.output_path,
        &mut session,
        complete && !fallback_required,
        fallback_required,
        None,
    )
}

fn start_segment_cleanup_worker(
    args: &Args,
    worker: &IncrementalWorkerState,
    state_file: &Path,
    stopping_path: &Path,
    recorder_pid: u32,
    local_only: bool,
) -> SegmentCleanupWorker {
    let queue_size = incremental_config().max_queue.max(1);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<TranscriptSegment>(queue_size);
    let (results_tx, results_rx) = mpsc::channel::<TranscriptSegment>();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let args = args.clone();
    let worker = worker.clone();
    let state_file = state_file.to_path_buf();
    let stopping_path = stopping_path.to_path_buf();
    let handle = thread::spawn(move || {
        while let Ok(raw) = jobs_rx.recv() {
            if worker_cancel.load(Ordering::SeqCst)
                || segment_cleanup_cancelled(&worker, &state_file, &stopping_path, recorder_pid)
            {
                break;
            }

            let result = cleanup_segment_text(&args, &raw.raw_text, local_only, || {
                worker_cancel.load(Ordering::SeqCst)
                    || segment_cleanup_cancelled(&worker, &state_file, &stopping_path, recorder_pid)
            });
            if worker_cancel.load(Ordering::SeqCst)
                || segment_cleanup_cancelled(&worker, &state_file, &stopping_path, recorder_pid)
            {
                break;
            }

            let segment = match result {
                Ok(Some(cleaned)) => TranscriptSegment {
                    status: TranscriptSegmentStatus::Cleaned,
                    cleaned_text: Some(cleaned),
                    ..raw
                },
                Ok(None) | Err(_) => raw,
            };
            if results_tx.send(segment).is_err() {
                break;
            }
        }
    });

    SegmentCleanupWorker {
        cancel,
        jobs_tx: Some(jobs_tx),
        results_rx,
        handle: Some(handle),
    }
}

fn segment_cleanup_cancelled(
    worker: &IncrementalWorkerState,
    state_file: &Path,
    stopping_path: &Path,
    recorder_pid: u32,
) -> bool {
    worker.cancel_path.exists()
        || (!worker.stop_path.exists()
            && !stopping_path.exists()
            && (!state_file.exists() || !pid_alive(recorder_pid)))
}

fn queue_segment_cleanup(
    worker: &IncrementalWorkerState,
    session: &mut RecordingSession,
    cleanup_worker: Option<&SegmentCleanupWorker>,
    raw: TranscriptSegment,
    fallback_required: bool,
) -> Result<()> {
    if raw.raw_text.trim().is_empty() {
        return Ok(());
    }
    let Some(cleanup_worker) = cleanup_worker else {
        return Ok(());
    };

    let Some(jobs_tx) = cleanup_worker.jobs_tx.as_ref() else {
        return Ok(());
    };

    if jobs_tx.try_send(raw.clone()).is_ok() {
        session.upsert_segment(TranscriptSegment {
            status: TranscriptSegmentStatus::Cleaning,
            ..raw
        });
        write_incremental_output(&worker.output_path, session, false, fallback_required, None)?;
    }
    Ok(())
}

fn drain_segment_cleanup_results(
    worker: &IncrementalWorkerState,
    session: &mut RecordingSession,
    cleanup_worker: Option<&SegmentCleanupWorker>,
    fallback_required: bool,
) -> Result<()> {
    let Some(cleanup_worker) = cleanup_worker else {
        return Ok(());
    };

    let mut changed = false;
    while let Ok(segment) = cleanup_worker.results_rx.try_recv() {
        session.upsert_segment(segment);
        changed = true;
    }
    if changed {
        write_incremental_output(&worker.output_path, session, false, fallback_required, None)?;
    }
    Ok(())
}

fn stop_segment_cleanup_worker(cleanup_worker: Option<SegmentCleanupWorker>) {
    drop(cleanup_worker);
}

fn incremental_transcript(worker: Option<&IncrementalWorkerState>) -> Option<String> {
    let worker = worker?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match read_incremental_output(&worker.output_path) {
            Ok(Some(output)) => {
                if let Some(text) = final_incremental_text(worker, &output) {
                    wait_then_terminate_incremental_worker(worker);
                    return Some(text);
                }
                if output.fallback_required || output.error.is_some() {
                    break;
                }
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if worker.pid != 0 && !worker_process_alive(worker) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    signal_incremental_cancel(Some(worker));
    terminate_incremental_worker(Some(worker));
    None
}

fn final_incremental_text(
    worker: &IncrementalWorkerState,
    output: &IncrementalWorkerOutput,
) -> Option<String> {
    if output.session.id != worker.session_id || !output.complete || output.fallback_required {
        return None;
    }
    let text = output.session.final_raw_text();
    (!text.trim().is_empty()).then_some(text)
}

fn incremental_segment_cleanup_enabled(args: &Args) -> bool {
    !args.no_llm
        && (args.stdout_only || args.no_paste)
        && (args.incremental_cleanup || env_flag_enabled("PICKSCRIBE_INCREMENTAL_CLEANUP"))
}

fn signal_incremental_stop(worker: Option<&IncrementalWorkerState>) {
    if let Some(worker) = worker {
        let _ = fs::write(&worker.stop_path, unix_secs().to_string());
    }
}

fn signal_incremental_stopping(worker: Option<&IncrementalWorkerState>) {
    if let Some(worker) = worker {
        let _ = fs::write(incremental_stopping_path(worker), unix_secs().to_string());
    }
}

fn signal_incremental_cancel(worker: Option<&IncrementalWorkerState>) {
    if let Some(worker) = worker {
        let _ = fs::write(&worker.cancel_path, unix_secs().to_string());
    }
}

fn incremental_stopping_path(worker: &IncrementalWorkerState) -> PathBuf {
    worker.temp_dir.join("stopping")
}

fn wait_then_terminate_incremental_worker(worker: &IncrementalWorkerState) {
    if worker.pid == 0 {
        return;
    }
    wait_for_worker_exit(worker, Duration::from_secs(1));
    if worker_process_alive(worker) {
        terminate_incremental_worker(Some(worker));
    }
}

fn terminate_incremental_worker(worker: Option<&IncrementalWorkerState>) {
    let Some(worker) = worker else {
        return;
    };
    if !worker_signal_target_alive(worker) {
        return;
    }
    wait_for_worker_exit(worker, Duration::from_secs(10));
    if !worker_signal_target_alive(worker) || incremental_worker_output_finished(worker) {
        return;
    }
    let _ = send_signal(worker.pid, "INT");
    wait_for_worker_exit(worker, Duration::from_secs(1));
    if worker_signal_target_alive(worker) {
        let _ = send_signal(worker.pid, "TERM");
        wait_for_worker_exit(worker, Duration::from_secs(1));
    }
    if worker_signal_target_alive(worker) {
        let _ = send_signal(worker.pid, "KILL");
        wait_for_worker_exit(worker, Duration::from_secs(1));
    }
}

fn worker_process_alive(worker: &IncrementalWorkerState) -> bool {
    if worker.pid == 0 {
        return false;
    }

    match worker.worker_started_ticks {
        Some(expected) => process_start_ticks(worker.pid) == Some(expected),
        None => pid_alive(worker.pid),
    }
}

fn worker_signal_target_alive(worker: &IncrementalWorkerState) -> bool {
    if worker.pid == 0 {
        return false;
    }

    worker
        .worker_started_ticks
        .is_some_and(|expected| process_start_ticks(worker.pid) == Some(expected))
}

fn wait_for_worker_exit(worker: &IncrementalWorkerState, timeout: Duration) {
    let start = SystemTime::now();
    while worker_process_alive(worker) {
        if start.elapsed().unwrap_or_default() >= timeout {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn incremental_worker_output_finished(worker: &IncrementalWorkerState) -> bool {
    read_incremental_output(&worker.output_path)
        .ok()
        .flatten()
        .map(|output| output.complete || output.fallback_required || output.error.is_some())
        .unwrap_or(false)
}

fn read_incremental_output(path: &Path) -> Result<Option<IncrementalWorkerOutput>> {
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&data)
        .map(Some)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn write_incremental_output(
    path: &Path,
    session: &mut RecordingSession,
    complete: bool,
    fallback_required: bool,
    error: Option<String>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let output = IncrementalWorkerOutput {
        session: session.clone(),
        complete,
        fallback_required,
        error,
        updated_unix_secs: unix_secs(),
    };
    let tmp_path = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(&output).context("failed to serialize output")?;
    fs::write(&tmp_path, data)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| format!("failed to write {}", path.display()))
}

fn atomic_write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.json");
    let tmp_path = path.with_file_name(format!(".{filename}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp_path)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    file.write_all(data)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to flush {}", tmp_path.display()))?;
    drop(file);
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| format!("failed to write {}", path.display()))
}

fn cleanup_incremental_files(args: &Args, worker: Option<&IncrementalWorkerState>) {
    let Some(worker) = worker else {
        return;
    };
    if args.keep_audio {
        return;
    }
    let _ = fs::remove_dir_all(&worker.temp_dir);
}

fn growing_audio_duration_ms(path: &Path) -> Result<u64> {
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

fn transcribe(args: &Args, audio_path: &Path) -> Result<String> {
    let output_prefix = transcript_prefix_for(audio_path);

    if let Some(custom) = args
        .stt_command
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let model = args
            .whisper_model
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let command = custom
            .replace("{audio}", &shell_escape(&audio_path.display().to_string()))
            .replace("{model}", &shell_escape(&model))
            .replace(
                "{output}",
                &shell_escape(&output_prefix.display().to_string()),
            );

        let output = Command::new("sh")
            .arg("-lc")
            .arg(&command)
            .output()
            .with_context(|| format!("failed to run custom STT command: {command}"))?;

        if !output.status.success() {
            bail!(
                "custom STT command failed with {}:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let txt_path = transcript_txt_path_for(audio_path);
        if txt_path.exists() {
            return fs::read_to_string(&txt_path)
                .with_context(|| format!("failed to read {}", txt_path.display()));
        }

        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let whisper = resolve_whisper_command(args)?;
    let mut cmd = Command::new(&whisper.program);

    if let Some(model) = whisper.model.as_ref() {
        cmd.arg("--model").arg(model);
    }

    cmd.arg("--file")
        .arg(audio_path)
        .arg("--output-txt")
        .arg("--output-file")
        .arg(&output_prefix)
        .arg("--no-prints");

    if let Some(language) = args.language.as_deref().filter(|value| !value.is_empty()) {
        cmd.arg("--language").arg(language);
    }

    let output = cmd
        .output()
        .with_context(|| format!("failed to run `{}`", whisper.program.display()))?;

    if !output.status.success() {
        bail!(
            "whisper.cpp failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let txt_path = transcript_txt_path_for(audio_path);
    if txt_path.exists() {
        fs::read_to_string(&txt_path)
            .with_context(|| format!("failed to read {}", txt_path.display()))
    } else {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

fn transcribe_incremental_segment(
    args: &Args,
    audio_path: &Path,
    is_cancelled: impl Fn() -> bool,
) -> Result<String> {
    let output_prefix = transcript_prefix_for(audio_path);

    if let Some(custom) = args
        .stt_command
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let model = args
            .whisper_model
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let command = custom
            .replace("{audio}", &shell_escape(&audio_path.display().to_string()))
            .replace("{model}", &shell_escape(&model))
            .replace(
                "{output}",
                &shell_escape(&output_prefix.display().to_string()),
            );
        let stdout_path = output_prefix.with_extension("transcript.stdout.log");
        let stderr_path = output_prefix.with_extension("transcript.stderr.log");
        let stdout_file = File::create(&stdout_path)
            .with_context(|| format!("failed to create {}", stdout_path.display()))?;
        let stderr_file = File::create(&stderr_path)
            .with_context(|| format!("failed to create {}", stderr_path.display()))?;
        let (mut cmd, process_group) = cancellable_shell_command();
        cmd.arg("-lc")
            .arg(&command)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to run custom STT command: {command}"))?;
        let status = match wait_for_cancellable_child(&mut child, process_group, is_cancelled) {
            Ok(status) => status,
            Err(err) => {
                cleanup_segment_transcript_files(audio_path);
                return Err(err);
            }
        };
        if !status.success() {
            let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
            cleanup_segment_transcript_files(audio_path);
            bail!(
                "custom STT command failed with {}:\n{}",
                status,
                stderr.trim()
            );
        }

        let txt_path = transcript_txt_path_for(audio_path);
        let output = if txt_path.exists() {
            fs::read_to_string(&txt_path)
                .with_context(|| format!("failed to read {}", txt_path.display()))?
        } else {
            fs::read_to_string(&stdout_path)
                .with_context(|| format!("failed to read {}", stdout_path.display()))?
        };
        let _ = fs::remove_file(&stdout_path);
        let _ = fs::remove_file(&stderr_path);
        return Ok(output);
    }

    let whisper = resolve_whisper_command(args)?;
    let stderr_path = output_prefix.with_extension("transcript.stderr.log");
    let stderr_file = File::create(&stderr_path)
        .with_context(|| format!("failed to create {}", stderr_path.display()))?;
    let (mut cmd, process_group) = cancellable_program_command(&whisper.program);

    if let Some(model) = whisper.model.as_ref() {
        cmd.arg("--model").arg(model);
    }

    cmd.arg("--file")
        .arg(audio_path)
        .arg("--output-txt")
        .arg("--output-file")
        .arg(&output_prefix)
        .arg("--no-prints");

    if let Some(language) = args.language.as_deref().filter(|value| !value.is_empty()) {
        cmd.arg("--language").arg(language);
    }

    cmd.stdout(Stdio::null()).stderr(Stdio::from(stderr_file));
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to run `{}`", whisper.program.display()))?;
    let status = match wait_for_cancellable_child(&mut child, process_group, is_cancelled) {
        Ok(status) => status,
        Err(err) => {
            cleanup_segment_transcript_files(audio_path);
            return Err(err);
        }
    };
    if !status.success() {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        cleanup_segment_transcript_files(audio_path);
        bail!("whisper.cpp failed with {}:\n{}", status, stderr.trim());
    }

    let _ = fs::remove_file(&stderr_path);
    let txt_path = transcript_txt_path_for(audio_path);
    if txt_path.exists() {
        fs::read_to_string(&txt_path)
            .with_context(|| format!("failed to read {}", txt_path.display()))
    } else {
        Ok(String::new())
    }
}

fn cancellable_shell_command() -> (Command, bool) {
    if let Some(setsid) = find_command("setsid") {
        let mut cmd = Command::new(setsid);
        cmd.arg("sh");
        (cmd, true)
    } else {
        (Command::new("sh"), false)
    }
}

fn cancellable_program_command(program: &Path) -> (Command, bool) {
    if let Some(setsid) = find_command("setsid") {
        let mut cmd = Command::new(setsid);
        cmd.arg(program);
        (cmd, true)
    } else {
        (Command::new(program), false)
    }
}

fn wait_for_cancellable_child(
    child: &mut Child,
    process_group: bool,
    is_cancelled: impl Fn() -> bool,
) -> Result<ExitStatus> {
    loop {
        if is_cancelled() {
            terminate_child(child, process_group);
            bail!("transcription cancelled");
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn terminate_child(child: &mut Child, process_group: bool) {
    let pid = child.id();
    if process_group {
        let _ = send_process_group_signal(pid, "INT");
        wait_for_exit(pid, Duration::from_secs(1));
        if pid_alive(pid) {
            let _ = send_process_group_signal(pid, "TERM");
            wait_for_exit(pid, Duration::from_secs(1));
        }
        if pid_alive(pid) {
            let _ = send_process_group_signal(pid, "KILL");
        }
    } else {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn send_process_group_signal(pid: u32, signal: &str) -> Result<()> {
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(format!("-{pid}"))
        .status()
        .with_context(|| format!("failed to send SIG{signal} to process group {pid}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("kill -{signal} -{pid} exited with {status}");
    }
}

fn cleanup_segment_transcript_files(audio_path: &Path) {
    let prefix = transcript_prefix_for(audio_path);
    let _ = fs::remove_file(transcript_txt_path_for(audio_path));
    let _ = fs::remove_file(prefix.with_extension("transcript.stdout.log"));
    let _ = fs::remove_file(prefix.with_extension("transcript.stderr.log"));
}

#[derive(Debug)]
struct WhisperCommand {
    program: PathBuf,
    model: Option<PathBuf>,
}

fn resolve_whisper_command(args: &Args) -> Result<WhisperCommand> {
    if let Some(model) = args.whisper_model.as_ref() {
        if !model.exists() {
            bail!("Whisper model does not exist: {}", model.display());
        }
        let program = find_command("whisper.cpp")
            .or_else(|| find_command("whisper-cli"))
            .ok_or_else(install_hint)?;
        return Ok(WhisperCommand {
            program,
            model: Some(model.clone()),
        });
    }

    if let Some(program) = find_command("whisper.cpp") {
        if let Some(model) = detect_model_path() {
            return Ok(WhisperCommand {
                program,
                model: Some(model),
            });
        }
    }

    for wrapper in [
        "whisper.cpp-base.en",
        "whisper.cpp-base",
        "whisper.cpp-small.en",
        "whisper.cpp-small",
        "whisper.cpp-tiny.en",
        "whisper.cpp-tiny",
        "whisper.cpp-large-v3-turbo-q5_0",
        "whisper.cpp-large-v3-turbo",
    ] {
        if let Some(program) = find_command(wrapper) {
            return Ok(WhisperCommand {
                program,
                model: None,
            });
        }
    }

    Err(install_hint())
}

fn install_hint() -> anyhow::Error {
    anyhow!(
        "no whisper.cpp backend/model found. Recommended local install:\n  scripts/install-whisper-cpp-local\n\nAUR alternative, if it builds on your system:\n  yay -S whisper.cpp whisper.cpp-model-base\n# English-only, faster but no Portuguese:\n  yay -S whisper.cpp whisper.cpp-model-base.en\n\nThen test:\n  pickscribe start\n  # speak\n  pickscribe stop --stdout-only"
    )
}

fn detect_model_path() -> Option<PathBuf> {
    // Prefer multilingual models so auto-detect works for English + Portuguese.
    // The *.en models are English-only and should only be used when no multilingual
    // model is installed or when PICKSCRIBE_WHISPER_MODEL points to them explicitly.
    let mut candidates = Vec::new();

    if let Some(home) = env::var_os("HOME") {
        let model_dir = PathBuf::from(home).join(".local/share/whisper.cpp/models");
        candidates.extend([
            model_dir.join("ggml-base.bin"),
            model_dir.join("ggml-small.bin"),
            model_dir.join("ggml-tiny.bin"),
            model_dir.join("ggml-large-v3-turbo-q5_0.bin"),
            model_dir.join("ggml-large-v3-turbo.bin"),
            model_dir.join("ggml-base.en.bin"),
            model_dir.join("ggml-small.en.bin"),
            model_dir.join("ggml-tiny.en.bin"),
        ]);
    }

    candidates.extend([
        PathBuf::from("/usr/share/whisper.cpp-model-base/ggml-base.bin"),
        PathBuf::from("/usr/share/whisper.cpp-model-small/ggml-small.bin"),
        PathBuf::from("/usr/share/whisper.cpp-model-tiny/ggml-tiny.bin"),
        PathBuf::from(
            "/usr/share/whisper.cpp-model-large-v3-turbo-q5_0/ggml-large-v3-turbo-q5_0.bin",
        ),
        PathBuf::from("/usr/share/whisper.cpp-model-large-v3-turbo/ggml-large-v3-turbo.bin"),
        PathBuf::from("/usr/share/whisper.cpp-model-base.en/ggml-base.en.bin"),
        PathBuf::from("/usr/share/whisper.cpp-model-small.en/ggml-small.en.bin"),
        PathBuf::from("/usr/share/whisper.cpp-model-tiny.en/ggml-tiny.en.bin"),
    ]);

    candidates.into_iter().find(|path| path.exists())
}

const WHISPER_CPP_REPO_URL: &str = "https://github.com/ggml-org/whisper.cpp.git";

#[derive(Debug)]
struct WhisperUpdateInfo {
    source_dir: PathBuf,
    source_exists: bool,
    local_head: Option<String>,
    remote_head: Option<String>,
    cli_path: Option<PathBuf>,
    model_path: PathBuf,
    model_exists: bool,
    update_available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCheckState {
    last_checked_unix_secs: u64,
}

fn check_whisper(args: &Args) -> Result<()> {
    let info = whisper_update_info(args)?;

    println!("source: {}", info.source_dir.display());
    println!("source_exists: {}", info.source_exists);
    println!(
        "local_head: {}",
        info.local_head.as_deref().unwrap_or("not installed")
    );
    println!(
        "remote_head: {}",
        info.remote_head.as_deref().unwrap_or("unknown")
    );
    println!(
        "whisper_cli: {}",
        info.cli_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not found".to_owned())
    );
    println!("model: {}", info.model_path.display());
    println!("model_exists: {}", info.model_exists);

    if info.update_available {
        println!("status: update available or local install incomplete");
        println!("run: pickscribe-gui update-whisper");
    } else {
        println!("status: up to date");
    }

    write_update_check_state(args).ok();
    Ok(())
}

fn update_whisper(args: &Args) -> Result<()> {
    let source_dir = whisper_cpp_src_dir(args)?;
    let build_dir = source_dir.join("build");
    let bin_dir = local_bin_dir()?;
    let model_path = whisper_update_model_path(args)?;
    let model_dir = model_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("model path has no parent: {}", model_path.display()))?;

    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    fs::create_dir_all(&model_dir)
        .with_context(|| format!("failed to create {}", model_dir.display()))?;

    if let Some(parent) = source_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if source_dir.join(".git").is_dir() {
        println!("Updating whisper.cpp in {}...", source_dir.display());
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&source_dir).arg("pull").arg("--ff-only");
        run_status_command(&mut cmd, "git pull whisper.cpp")?;
    } else {
        if source_dir.exists() && fs::read_dir(&source_dir)?.next().is_some() {
            bail!(
                "{} exists but is not a git checkout; move it away or set PICKSCRIBE_WHISPER_CPP_SRC",
                source_dir.display()
            );
        }

        println!("Cloning whisper.cpp into {}...", source_dir.display());
        let mut cmd = Command::new("git");
        cmd.arg("clone")
            .arg("--depth")
            .arg("1")
            .arg(WHISPER_CPP_REPO_URL)
            .arg(&source_dir);
        run_status_command(&mut cmd, "git clone whisper.cpp")?;
    }

    println!("Configuring whisper.cpp...");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&source_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg("-G")
        .arg("Ninja")
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DWHISPER_BUILD_TESTS=OFF")
        .arg("-DWHISPER_BUILD_EXAMPLES=ON");
    run_status_command(&mut configure, "cmake configure whisper.cpp")?;

    println!("Building whisper.cpp...");
    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg("Release")
        .arg("--parallel")
        .arg(parallelism_string());
    run_status_command(&mut build, "cmake build whisper.cpp")?;

    let built_cli = if build_dir.join("bin/whisper-cli").is_file() {
        build_dir.join("bin/whisper-cli")
    } else if build_dir.join("bin/main").is_file() {
        build_dir.join("bin/main")
    } else {
        bail!(
            "no whisper CLI binary found under {}",
            build_dir.join("bin").display()
        );
    };

    let cli_link = bin_dir.join("whisper-cli");
    let _ = fs::remove_file(&cli_link);
    unix_fs::symlink(&built_cli, &cli_link).with_context(|| {
        format!(
            "failed to symlink {} -> {}",
            cli_link.display(),
            built_cli.display()
        )
    })?;

    if model_path.exists() && fs::metadata(&model_path)?.len() > 0 {
        println!("Model already exists: {}", model_path.display());
    } else {
        println!(
            "Downloading Whisper model `{}` into {}...",
            args.whisper_model_name,
            model_dir.display()
        );
        let script = source_dir.join("models/download-ggml-model.sh");
        let mut download = Command::new("bash");
        download
            .arg(&script)
            .arg(&args.whisper_model_name)
            .arg(&model_dir);
        run_status_command(&mut download, "download Whisper model")?;
    }

    update_pickscribe_env_file(args, &model_path)?;
    write_update_check_state(args).ok();

    println!("Updated whisper.cpp CLI: {}", cli_link.display());
    println!("Configured model: {}", model_path.display());
    Ok(())
}

fn maybe_auto_update_whisper(args: &Args) -> Result<()> {
    if args.auto_update_whisper == AutoUpdateWhisper::Off {
        return Ok(());
    }

    if !update_check_due(args)? {
        return Ok(());
    }

    let result = match args.auto_update_whisper {
        AutoUpdateWhisper::Off => Ok(()),
        AutoUpdateWhisper::Check => {
            let info = whisper_update_info(args)?;
            if info.update_available {
                notify(
                    args,
                    "PickScribe",
                    "whisper.cpp update available. Run pickscribe-gui update-whisper.",
                );
                eprintln!("whisper.cpp update available; run `pickscribe-gui update-whisper`");
            }
            Ok(())
        }
        AutoUpdateWhisper::Install => {
            let info = whisper_update_info(args)?;
            if info.update_available {
                notify(args, "PickScribe", "Updating whisper.cpp...");
                update_whisper(args)?;
                notify(args, "PickScribe", "whisper.cpp updated");
            }
            Ok(())
        }
    };

    write_update_check_state(args).ok();
    result
}

fn whisper_update_info(args: &Args) -> Result<WhisperUpdateInfo> {
    let source_dir = whisper_cpp_src_dir(args)?;
    let source_exists = source_dir.join(".git").is_dir();
    let model_path = whisper_update_model_path(args)?;
    let model_exists =
        model_path.exists() && fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0) > 0;
    let cli_path = find_command("whisper-cli")
        .or_else(|| find_command("whisper.cpp"))
        .or_else(|| {
            let built = source_dir.join("build/bin/whisper-cli");
            built.is_file().then_some(built)
        });

    let local_head = if source_exists {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&source_dir).arg("rev-parse").arg("HEAD");
        Some(command_stdout(&mut cmd, "get local whisper.cpp HEAD")?)
    } else {
        None
    };

    let remote_head = if source_exists {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&source_dir)
            .arg("ls-remote")
            .arg("origin")
            .arg("HEAD");
        parse_ls_remote_head(&command_stdout(&mut cmd, "get remote whisper.cpp HEAD")?)
    } else {
        let mut cmd = Command::new("git");
        cmd.arg("ls-remote").arg(WHISPER_CPP_REPO_URL).arg("HEAD");
        parse_ls_remote_head(&command_stdout(&mut cmd, "get remote whisper.cpp HEAD")?)
    };

    let update_available = !source_exists
        || cli_path.is_none()
        || !model_exists
        || matches!((&local_head, &remote_head), (Some(local), Some(remote)) if local != remote);

    Ok(WhisperUpdateInfo {
        source_dir,
        source_exists,
        local_head,
        remote_head,
        cli_path,
        model_path,
        model_exists,
        update_available,
    })
}

fn whisper_cpp_src_dir(args: &Args) -> Result<PathBuf> {
    if let Some(path) = args.whisper_cpp_src.as_ref() {
        return Ok(path.clone());
    }
    Ok(home_dir()?.join(".local/src/whisper.cpp"))
}

fn whisper_model_dir(args: &Args) -> Result<PathBuf> {
    if let Some(path) = args.whisper_model_dir.as_ref() {
        return Ok(path.clone());
    }
    Ok(home_dir()?.join(".local/share/whisper.cpp/models"))
}

fn whisper_update_model_path(args: &Args) -> Result<PathBuf> {
    if let Some(path) = args.whisper_model.as_ref() {
        return Ok(path.clone());
    }

    let name = args.whisper_model_name.trim();
    if name.is_empty() {
        bail!("PICKSCRIBE_WHISPER_MODEL_NAME cannot be empty");
    }

    Ok(whisper_model_dir(args)?.join(format!("ggml-{name}.bin")))
}

fn local_bin_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".local/bin"))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn persistent_state_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(dir).join("pickscribe"));
    }
    Ok(home_dir()?.join(".local/state/pickscribe"))
}

fn update_check_state_path() -> Result<PathBuf> {
    Ok(persistent_state_dir()?.join("whisper-update-check.json"))
}

fn update_check_due(args: &Args) -> Result<bool> {
    if args.update_interval_hours == 0 {
        return Ok(true);
    }

    let path = update_check_state_path()?;
    if !path.exists() {
        return Ok(true);
    }

    let data = fs::read_to_string(&path).unwrap_or_default();
    let state: UpdateCheckState = match serde_json::from_str(&data) {
        Ok(state) => state,
        Err(_) => return Ok(true),
    };

    let interval_secs = args
        .update_interval_hours
        .saturating_mul(60)
        .saturating_mul(60);
    Ok(unix_secs().saturating_sub(state.last_checked_unix_secs) >= interval_secs)
}

fn write_update_check_state(_args: &Args) -> Result<()> {
    let path = update_check_state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let state = UpdateCheckState {
        last_checked_unix_secs: unix_secs(),
    };
    let data = serde_json::to_string_pretty(&state).context("failed to serialize update state")?;
    fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))
}

fn update_pickscribe_env_file(args: &Args, model_path: &Path) -> Result<()> {
    let env_file = env::var_os("PICKSCRIBE_ENV_FILE")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config/pickscribe/env"));

    if let Some(parent) = env_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let existing = fs::read_to_string(&env_file).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| {
            let trimmed = line
                .trim_start()
                .strip_prefix("export ")
                .unwrap_or_else(|| line.trim_start());
            !trimmed.starts_with("PICKSCRIBE_LANGUAGE=")
                && !trimmed.starts_with("PICKSCRIBE_WHISPER_MODEL=")
                && !trimmed.starts_with("PICKSCRIBE_WHISPER_MODEL_NAME=")
        })
        .map(ToOwned::to_owned)
        .collect();

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.push(String::new());
    }

    lines.push("PICKSCRIBE_LANGUAGE=\"auto\"".to_owned());
    lines.push(format!(
        "PICKSCRIBE_WHISPER_MODEL=\"{}\"",
        escape_env_double_quoted(&model_path.display().to_string())
    ));
    lines.push(format!(
        "PICKSCRIBE_WHISPER_MODEL_NAME=\"{}\"",
        escape_env_double_quoted(&args.whisper_model_name)
    ));

    fs::write(&env_file, format!("{}\n", lines.join("\n")))
        .with_context(|| format!("failed to write {}", env_file.display()))?;
    Ok(())
}

fn command_stdout(command: &mut Command, description: &str) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if !output.status.success() {
        bail!(
            "{description} failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_status_command(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to run {description}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{description} failed with {status}")
    }
}

fn parse_ls_remote_head(output: &str) -> Option<String> {
    output.split_whitespace().next().map(ToOwned::to_owned)
}

fn parallelism_string() -> String {
    thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2)
        .to_string()
}

fn escape_env_double_quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn resolve_cleanup_command(args: &Args) -> Result<PathBuf> {
    args.cleanup_command
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| find_command("pickscribe-cleanup-gui"))
        .or_else(|| find_command("pickscribe-cleanup"))
        .or_else(|| find_command("voice-cleanup-gui"))
        .or_else(|| find_command("voice-cleanup"))
        .ok_or_else(|| anyhow!("pickscribe-cleanup not found; install this project first"))
}

fn final_cleanup_args(args: &Args) -> Vec<OsString> {
    let mut cleanup_args: Vec<OsString> = Vec::new();
    if args.stdout_only {
        cleanup_args.push("--stdout-only".into());
    } else {
        if args.no_copy {
            cleanup_args.push("--no-copy".into());
        }
        if args.no_paste {
            cleanup_args.push("--no-paste".into());
        }
        if args.print {
            cleanup_args.push("--print".into());
        }
    }
    if args.no_llm {
        cleanup_args.push("--no-llm".into());
    }
    cleanup_args
}

fn run_cleanup(args: &Args, transcript: &str) -> Result<()> {
    let cleanup = resolve_cleanup_command(args)?;
    let cleanup_args = final_cleanup_args(args);

    let mut child = Command::new(&cleanup)
        .args(cleanup_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to start cleanup command `{}`", cleanup.display()))?;

    child
        .stdin
        .take()
        .context("failed to open cleanup stdin")?
        .write_all(transcript.as_bytes())
        .context("failed to send transcript to cleanup command")?;

    let status = child.wait().context("failed to wait for cleanup command")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cleanup command exited with {status}");
    }
}

fn cleanup_segment_text(
    args: &Args,
    transcript: &str,
    local_only: bool,
    is_cancelled: impl Fn() -> bool,
) -> Result<Option<String>> {
    let cleanup_command = resolve_cleanup_command(args)?;
    let bundled_cleanup = args.cleanup_command.is_none()
        && cleanup_command
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "pickscribe-cleanup" | "pickscribe-cleanup-gui"));
    if local_only && args.cleanup_command.is_some() {
        bail!("local-only mode blocks custom CLI segment cleanup commands");
    }
    let mut cleanup_args: Vec<OsString> = vec!["--stdout-only".into()];
    if bundled_cleanup {
        cleanup_args.push("--segment".into());
    }
    if local_only {
        cleanup_args.push("--local-only".into());
    }
    if args.no_llm {
        cleanup_args.push("--no-llm".into());
    }

    let (mut cmd, process_group) = cancellable_program_command(&cleanup_command);
    cmd.args(cleanup_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "failed to start cleanup command `{}`",
            cleanup_command.display()
        )
    })?;

    {
        let mut stdin = child.stdin.take().context("failed to open cleanup stdin")?;
        stdin
            .write_all(transcript.as_bytes())
            .context("failed to send transcript to cleanup command")?;
    }

    let stdout = child
        .stdout
        .take()
        .context("failed to open cleanup stdout")?;
    let stdout_reader = thread::spawn(move || -> Result<String> {
        let mut child_stdout = stdout;
        let mut output = String::new();
        child_stdout
            .read_to_string(&mut output)
            .context("failed to read cleanup stdout")?;
        Ok(output)
    });
    let status = wait_for_cancellable_child(&mut child, process_group, is_cancelled)
        .context("failed to wait for cleanup command")?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("cleanup stdout reader panicked"))??;
    if !status.success() {
        bail!("cleanup command exited with {}", status);
    }
    let cleaned = stdout.trim().to_string();
    Ok((!cleaned.is_empty()
        && cleaned.trim() != transcript.trim()
        && cleanup_engine::segment_cleanup_is_safe(transcript, &cleaned))
    .then_some(cleaned))
}

fn cleanup_transcript(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if let Some(end) = trimmed.find(']') {
                if trimmed.starts_with('[') && trimmed[..=end].contains("-->") {
                    return trimmed[end + 1..].trim();
                }
            }
            trimmed
        })
        .filter(|line| !line.is_empty() && !is_non_speech_marker(line))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_non_speech_marker(line: &str) -> bool {
    matches!(
        line.trim().to_ascii_uppercase().as_str(),
        "[BLANK_AUDIO]"
            | "[MUSIC]"
            | "[SILENCE]"
            | "[NO SPEECH]"
            | "[INAUDIBLE]"
            | "(BLANK_AUDIO)"
            | "(MUSIC)"
            | "(SILENCE)"
            | "(NO SPEECH)"
            | "(INAUDIBLE)"
    )
}

fn state_dir(args: &Args) -> Result<PathBuf> {
    if let Some(dir) = args.state_dir.as_ref() {
        return Ok(dir.clone());
    }
    if let Some(dir) = env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir).join("pickscribe"));
    }
    let user = env::var("USER").unwrap_or_else(|_| "user".to_owned());
    Ok(PathBuf::from(format!("/tmp/pickscribe-{user}")))
}

fn prepare_state_dir(args: &Args, path: &Path) -> Result<()> {
    if args.state_dir.is_some() {
        fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
    } else {
        ensure_private_dir(path)
    }
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .permissions();
    let mode = permissions.mode();
    let private_mode = mode & !0o077;
    if private_mode != mode {
        permissions.set_mode(private_mode);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to restrict {}", path.display()))?;
    }
    Ok(())
}

fn state_path(args: &Args) -> Result<PathBuf> {
    Ok(state_dir(args)?.join("recording.json"))
}

fn transcript_prefix_for(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("transcript")
}

fn transcript_txt_path_for(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("transcript.txt")
}

fn read_active_state(args: &Args) -> Result<Option<RecordingState>> {
    let path = state_path(args)?;
    let Some(state) = read_state_file(&path)? else {
        return Ok(None);
    };
    if pid_alive(state.pid) {
        Ok(Some(state))
    } else {
        signal_incremental_cancel(state.incremental.as_ref());
        terminate_incremental_worker(state.incremental.as_ref());
        cleanup_incremental_files(args, state.incremental.as_ref());
        let _ = fs::remove_file(path);
        Ok(None)
    }
}

fn read_state_file(path: &Path) -> Result<Option<RecordingState>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read state file {}", path.display()))?;
    let state = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse state file {}", path.display()))?;
    Ok(Some(state))
}

fn write_state(args: &Args, state: &RecordingState) -> Result<()> {
    let path = state_path(args)?;
    write_state_file(&path, state)
}

fn write_state_file(path: &Path, state: &RecordingState) -> Result<()> {
    let data = serde_json::to_string_pretty(state).context("failed to serialize state")?;
    atomic_write_private(path, data.as_bytes())
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn pid_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

fn process_start_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("stat")).ok()?;
    parse_process_start_ticks(&stat)
}

fn parse_process_start_ticks(stat: &str) -> Option<u64> {
    let rest = stat.rsplit_once(") ")?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn send_signal(pid: u32, signal: &str) -> Result<()> {
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("failed to send SIG{signal} to pid {pid}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("kill -{signal} {pid} exited with {status}");
    }
}

fn stop_recorder(pid: u32) -> Result<()> {
    if pid_alive(pid) {
        send_signal(pid, "INT")?;
        wait_for_exit(pid, Duration::from_secs(5));
    }

    if pid_alive(pid) {
        send_signal(pid, "TERM")?;
        wait_for_exit(pid, Duration::from_secs(2));
    }

    if pid_alive(pid) {
        send_signal(pid, "KILL")?;
        wait_for_exit(pid, Duration::from_secs(1));
    }

    Ok(())
}

fn wait_for_exit(pid: u32, timeout: Duration) {
    let start = SystemTime::now();
    while pid_alive(pid) {
        if start.elapsed().unwrap_or_default() >= timeout {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn find_command(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if program.contains('/') {
        return path.is_file().then(|| path.to_path_buf());
    }

    pickscribe::engine::find_command(program)
}

fn notify(args: &Args, title: &str, body: &str) {
    if args.no_notify {
        return;
    }
    if let Some(program) = find_command("notify-send") {
        let _ = Command::new(program)
            .arg(title)
            .arg(body)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn cleanup_files(args: &Args, state: &RecordingState, transcript_path: Option<&Path>) {
    if args.keep_audio {
        return;
    }
    let _ = fs::remove_file(&state.audio_path);
    let _ = fs::remove_file(&state.log_path);
    if let Some(path) = transcript_path {
        let _ = fs::remove_file(path);
    }
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn parse_args(values: &[&str]) -> Args {
        Args::parse_from(values)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("pickscribe-{name}-{id}"))
    }

    fn worker_state(session_id: &str) -> IncrementalWorkerState {
        let temp_dir = PathBuf::from("/tmp/pickscribe-test").join(session_id);
        IncrementalWorkerState {
            pid: 0,
            worker_started_ticks: None,
            session_id: session_id.to_string(),
            output_path: temp_dir.join("worker-output.json"),
            stop_path: temp_dir.join("stop"),
            cancel_path: temp_dir.join("cancel"),
            log_path: temp_dir.join("worker.log"),
            temp_dir,
        }
    }

    #[test]
    fn cleanup_transcript_strips_whisper_timestamps_and_markers() {
        let transcript = "
            [00:00:00.000 --> 00:00:01.000] hello there
            [BLANK_AUDIO]
            [00:00:01.000 --> 00:00:02.000] general kenobi
            (music)
        ";

        assert_eq!(cleanup_transcript(transcript), "hello there general kenobi");
    }

    #[test]
    fn cleanup_transcript_trims_and_joins_plain_lines() {
        let transcript = " first line\n\nsecond line \n[inaudible]\n";

        assert_eq!(cleanup_transcript(transcript), "first line second line");
    }

    #[test]
    fn incremental_segment_cleanup_requires_opt_in_and_llm_cleanup() {
        let args = parse_args(&["pickscribe"]);
        assert!(!incremental_segment_cleanup_enabled(&args));

        let args = parse_args(&["pickscribe", "--incremental-cleanup"]);
        assert!(!incremental_segment_cleanup_enabled(&args));

        let args = parse_args(&["pickscribe", "--incremental-cleanup", "--stdout-only"]);
        assert!(incremental_segment_cleanup_enabled(&args));

        let args = parse_args(&["pickscribe", "--incremental-cleanup", "--no-paste"]);
        assert!(incremental_segment_cleanup_enabled(&args));

        let args = parse_args(&[
            "pickscribe",
            "--incremental-cleanup",
            "--stdout-only",
            "--no-llm",
        ]);
        assert!(!incremental_segment_cleanup_enabled(&args));
    }

    #[test]
    fn cleanup_segment_text_uses_stdout_only_cleanup_command() -> Result<()> {
        let dir = temp_dir("segment-cleanup");
        fs::create_dir_all(&dir)?;
        let script = dir.join("cleanup.sh");
        fs::write(
            &script,
            "#!/usr/bin/env bash\nwhile [[ ${1-} == --* ]]; do shift; done\ntr '[:lower:]' '[:upper:]'\n",
        )?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        let args = parse_args(&["pickscribe", "--cleanup-command", script.to_str().unwrap()]);

        let cleaned = cleanup_segment_text(&args, "hello segment", false, || false)?;

        assert_eq!(cleaned.as_deref(), Some("HELLO SEGMENT"));
        let _ = fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn cleanup_segment_text_ignores_exact_raw_fallback() -> Result<()> {
        let dir = temp_dir("segment-cleanup-raw-fallback");
        fs::create_dir_all(&dir)?;
        let script = dir.join("cleanup.sh");
        fs::write(
            &script,
            "#!/usr/bin/env bash\nwhile [[ ${1-} == --* ]]; do shift; done\ncat\n",
        )?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        let args = parse_args(&["pickscribe", "--cleanup-command", script.to_str().unwrap()]);

        let cleaned = cleanup_segment_text(&args, "hello segment", false, || false)?;

        assert!(cleaned.is_none());
        let _ = fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn cleanup_segment_text_blocks_custom_commands_in_local_only_mode() -> Result<()> {
        let args = parse_args(&["pickscribe", "--cleanup-command", "/bin/true"]);

        let err = cleanup_segment_text(&args, "hello segment", true, || false)
            .unwrap_err()
            .to_string();

        assert!(err.contains("local-only mode blocks custom CLI segment cleanup commands"));
        Ok(())
    }

    #[test]
    fn cleanup_segment_text_drains_large_stdout() -> Result<()> {
        let dir = temp_dir("segment-cleanup-large-stdout");
        fs::create_dir_all(&dir)?;
        let script = dir.join("cleanup.sh");
        fs::write(
            &script,
            "#!/usr/bin/env bash\npython3 - <<'PY'\nprint('x' * 200000)\nPY\n",
        )?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        let args = parse_args(&["pickscribe", "--cleanup-command", script.to_str().unwrap()]);

        let cleaned = cleanup_segment_text(&args, "hello segment", false, || false)?;

        assert!(cleaned.is_none());
        let _ = fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn recording_state_deserializes_legacy_state_files() {
        let state: RecordingState = serde_json::from_str(
            r#"{
                "pid": 123,
                "audio_path": "/tmp/recording.wav",
                "log_path": "/tmp/recording.log",
                "started_unix_secs": 456
            }"#,
        )
        .unwrap();

        assert_eq!(state.pid, 123);
        assert!(state.session_id.is_none());
        assert!(state.incremental.is_none());
    }

    #[test]
    fn process_start_ticks_parser_handles_spaced_command_names() {
        let fields = (3..=52)
            .map(|field| {
                if field == 22 {
                    "98765".to_owned()
                } else {
                    "0".to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let stat = format!("123 (pickscribe worker) {fields}");

        assert_eq!(parse_process_start_ticks(&stat), Some(98_765));
    }

    #[test]
    fn final_incremental_text_requires_complete_current_nonfallback_output() {
        let worker = worker_state("session-1");
        let mut session = RecordingSession::new("session-1");
        session.upsert_segment(TranscriptSegment::raw_ready(1, 0, 5_000, "hello world"));
        session.upsert_segment(TranscriptSegment::raw_ready(2, 4_000, 9_000, "world again"));
        let output = IncrementalWorkerOutput {
            session: session.clone(),
            complete: true,
            fallback_required: false,
            error: None,
            updated_unix_secs: 1,
        };

        assert_eq!(
            final_incremental_text(&worker, &output).as_deref(),
            Some("hello world again")
        );

        let mut incomplete = output.clone();
        incomplete.complete = false;
        assert!(final_incremental_text(&worker, &incomplete).is_none());

        let mut fallback = output.clone();
        fallback.fallback_required = true;
        assert!(final_incremental_text(&worker, &fallback).is_none());

        let stale = IncrementalWorkerOutput {
            session: RecordingSession::new("session-2"),
            ..output
        };
        assert!(final_incremental_text(&worker, &stale).is_none());
    }
}
