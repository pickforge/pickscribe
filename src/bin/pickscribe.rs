use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::Write,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Parser)]
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

#[derive(Debug, Serialize, Deserialize)]
struct RecordingState {
    pid: u32,
    audio_path: PathBuf,
    log_path: PathBuf,
    started_unix_secs: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

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
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

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

    let state = RecordingState {
        pid: child.id(),
        audio_path,
        log_path,
        started_unix_secs: stamp,
    };

    thread::sleep(Duration::from_millis(250));
    if !pid_alive(state.pid) {
        let log = fs::read_to_string(&state.log_path).unwrap_or_default();
        bail!("recorder exited immediately. Log:\n{log}");
    }

    write_state(args, &state)?;
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

    if pid_alive(state.pid) {
        send_signal(state.pid, "INT")?;
        wait_for_exit(state.pid, Duration::from_secs(5));
    }

    if pid_alive(state.pid) {
        send_signal(state.pid, "TERM")?;
        wait_for_exit(state.pid, Duration::from_secs(2));
    }

    if pid_alive(state.pid) {
        send_signal(state.pid, "KILL")?;
        wait_for_exit(state.pid, Duration::from_secs(1));
    }

    let _ = fs::remove_file(&state_path);

    if cancel {
        notify(args, "PickScribe", "Recording cancelled");
        cleanup_files(args, &state, None);
        println!("Recording cancelled.");
        return Ok(());
    }

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
    let transcript = transcribe(args, &state.audio_path)?;
    let transcript = cleanup_transcript(&transcript);

    if transcript.trim().is_empty() {
        notify(args, "PickScribe", "No speech detected");
        cleanup_files(args, &state, Some(&transcript_path));
        println!("No speech detected.");
        return Ok(());
    }

    notify(args, "PickScribe", "Cleaning and pasting...");
    println!("Cleaning and pasting...");
    run_cleanup(args, &transcript)?;
    notify(args, "PickScribe", "Done");

    cleanup_files(args, &state, Some(&transcript_path));
    Ok(())
}

fn print_status(args: &Args) -> Result<()> {
    match read_active_state(args)? {
        Some(state) => {
            println!("recording");
            println!("pid: {}", state.pid);
            println!("audio: {}", state.audio_path.display());
            println!("started: {}", state.started_unix_secs);
        }
        None => println!("idle"),
    }
    Ok(())
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

fn run_cleanup(args: &Args, transcript: &str) -> Result<()> {
    let cleanup = args
        .cleanup_command
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| find_command("pickscribe-cleanup-gui"))
        .or_else(|| find_command("pickscribe-cleanup"))
        .or_else(|| find_command("voice-cleanup-gui"))
        .or_else(|| find_command("voice-cleanup"))
        .ok_or_else(|| anyhow!("pickscribe-cleanup not found; install this project first"))?;

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
    let data = serde_json::to_string_pretty(state).context("failed to serialize state")?;
    fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))
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

    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
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
