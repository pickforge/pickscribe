use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::SttConfig;

pub struct WhisperSetup {
    pub program: PathBuf,
    pub model: PathBuf,
}

pub fn resolve_whisper(cfg: &SttConfig) -> Result<WhisperSetup> {
    let program = super::find_command("whisper-cli")
        .or_else(|| super::find_command("whisper.cpp"))
        .context("whisper-cli not found in PATH — run scripts/install-whisper-cpp-local")?;

    let model = if !cfg.model_path.is_empty() {
        let path = PathBuf::from(shellexpand_home(&cfg.model_path));
        if !path.is_file() {
            bail!("configured whisper model not found: {}", path.display());
        }
        path
    } else {
        detect_model_path().context(
            "no whisper model found — install one under ~/.local/share/whisper.cpp/models",
        )?
    };

    Ok(WhisperSetup { program, model })
}

pub fn detect_model_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PICKSCRIBE_WHISPER_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let model_dir = PathBuf::from(&home).join(".local/share/whisper.cpp/models");
    for name in [
        "ggml-large-v3-turbo.bin",
        "ggml-large-v3-turbo-q5_0.bin",
        "ggml-small.bin",
        "ggml-base.bin",
        "ggml-tiny.bin",
    ] {
        let candidate = model_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Arch whisper.cpp-model-* packages
    if let Ok(entries) = fs::read_dir("/usr/share") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("whisper.cpp-model") {
                if let Ok(files) = fs::read_dir(entry.path()) {
                    for file in files.flatten() {
                        let path = file.path();
                        if path.extension().is_some_and(|e| e == "bin") {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    None
}

/// List models available in the default model directory (for the settings UI).
pub fn available_models() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home).join(".local/share/whisper.cpp/models");
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().map(|n| n.to_string_lossy().to_string());
                if let Some(name) = name {
                    if name.starts_with("ggml-") && name.ends_with(".bin") {
                        out.push(path);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

pub fn transcribe(cfg: &SttConfig, audio: &Path) -> Result<String> {
    transcribe_with_cancel(cfg, audio, || false)
}

pub fn transcribe_with_cancel(
    cfg: &SttConfig,
    audio: &Path,
    is_cancelled: impl Fn() -> bool,
) -> Result<String> {
    let setup = resolve_whisper(cfg)?;
    let prefix = audio.with_extension("transcript");
    let mut cmd = Command::new(&setup.program);
    cmd.arg("--model")
        .arg(&setup.model)
        .arg("--file")
        .arg(audio)
        .arg("--output-txt")
        .arg("--output-file")
        .arg(&prefix)
        .arg("--no-prints");
    // whisper-cli defaults to --language en, which silently translates other
    // languages into English; auto-detection must be requested explicitly.
    let language = if cfg.language.is_empty() {
        "auto"
    } else {
        cfg.language.as_str()
    };
    cmd.arg("--language").arg(language);
    let stderr_path = prefix.with_extension("transcript.stderr.log");
    let stderr_file = fs::File::create(&stderr_path)
        .with_context(|| format!("creating transcript log {}", stderr_path.display()))?;
    cmd.stdout(Stdio::null()).stderr(Stdio::from(stderr_file));
    let mut child = cmd.spawn().context("running whisper-cli")?;
    let status: ExitStatus;
    loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(prefix.with_extension("transcript.txt"));
            let _ = fs::remove_file(&stderr_path);
            bail!("transcription cancelled");
        }
        if let Some(exit_status) = child.try_wait()? {
            status = exit_status;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !status.success() {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = fs::remove_file(&stderr_path);
        bail!("whisper-cli failed: {}", stderr.trim());
    }
    let _ = fs::remove_file(&stderr_path);
    let txt_path = prefix.with_extension("transcript.txt");
    let raw = fs::read_to_string(&txt_path)
        .with_context(|| format!("reading transcript {}", txt_path.display()))?;
    let _ = fs::remove_file(&txt_path);
    Ok(clean_transcript(&raw))
}

/// Strip whisper timestamps and non-speech markers like [MUSIC], (laughs).
pub fn clean_transcript(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let mut text = line.trim();
        if text.is_empty() {
            continue;
        }
        // Leading "[00:00:00.000 --> 00:00:05.000]" style timestamps.
        if text.starts_with('[') {
            if let Some(end) = text.find(']') {
                if text[..end].contains("-->") {
                    text = text[end + 1..].trim();
                }
            }
        }
        if text.is_empty() {
            continue;
        }
        let is_marker = (text.starts_with('[') && text.ends_with(']'))
            || (text.starts_with('(') && text.ends_with(')'));
        if is_marker {
            continue;
        }
        lines.push(text.to_string());
    }
    lines.join(" ").trim().to_string()
}

fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}
