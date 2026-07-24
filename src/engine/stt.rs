use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::SttConfig;
use crate::engine::transcript::{FileSegment, parse_whisper_json};

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
    if let Some(path) = detect_model_in_dir(&model_dir) {
        return Some(path);
    }
    // Arch whisper.cpp-model-* packages
    if let Ok(entries) = fs::read_dir("/usr/share") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("whisper.cpp-model")
                && let Ok(files) = fs::read_dir(entry.path())
            {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().is_some_and(|e| e == "bin") {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn detect_model_in_dir(model_dir: &Path) -> Option<PathBuf> {
    for name in [
        "ggml-large-v3-turbo.bin",
        "ggml-large-v3-turbo-q5_0.bin",
        "ggml-small.bin",
        "ggml-small.en.bin",
        "ggml-base.bin",
        "ggml-base.en.bin",
        "ggml-tiny.bin",
        "ggml-tiny.en.bin",
    ] {
        let candidate = model_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let mut models = fs::read_dir(model_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("ggml-"))
                && path.extension().is_some_and(|extension| extension == "bin")
        })
        .collect::<Vec<_>>();
    models.sort();
    models.into_iter().next()
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
                if let Some(name) = name
                    && name.starts_with("ggml-")
                    && name.ends_with(".bin")
                {
                    out.push(path);
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

pub fn transcribe_file_with_cancel(
    cfg: &SttConfig,
    wav: &Path,
    is_cancelled: impl Fn() -> bool,
    on_progress: impl Fn(u8),
) -> Result<Vec<FileSegment>> {
    let setup = resolve_whisper(cfg)?;
    let prefix = wav.with_extension("transcript");
    let stderr_path = prefix.with_extension("transcript.stderr.log");
    let stderr_file = fs::File::create(&stderr_path)
        .with_context(|| format!("creating transcript log {}", stderr_path.display()))?;
    let mut cmd = Command::new(&setup.program);
    cmd.arg("--model")
        .arg(&setup.model)
        .arg("--file")
        .arg(wav)
        .arg("--output-json")
        .arg("--output-file")
        .arg(&prefix)
        .arg("--print-progress")
        .arg("--no-prints");
    let language = if cfg.language.is_empty() {
        "auto"
    } else {
        cfg.language.as_str()
    };
    cmd.arg("--language").arg(language);
    cmd.stdout(Stdio::null()).stderr(Stdio::from(stderr_file));
    let mut child = match cmd.spawn().context("running whisper-cli") {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_file(&stderr_path);
            return Err(err);
        }
    };
    let mut last_progress = 0;
    let status: ExitStatus;
    loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(prefix.with_extension("transcript.json"));
            let _ = fs::remove_file(&stderr_path);
            bail!("transcription cancelled");
        }
        if let Some(progress) = read_log_tail(&stderr_path)
            .lines()
            .filter_map(parse_progress_percentage)
            .max()
            && progress > last_progress
        {
            last_progress = progress;
            on_progress(progress);
        }
        if let Some(exit_status) = child.try_wait()? {
            status = exit_status;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !status.success() {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = fs::remove_file(prefix.with_extension("transcript.json"));
        let _ = fs::remove_file(&stderr_path);
        bail!("whisper-cli failed: {}", stderr.trim());
    }

    let json_path = prefix.with_extension("transcript.json");
    let raw = fs::read_to_string(&json_path)
        .with_context(|| format!("reading transcript {}", json_path.display()));
    let _ = fs::remove_file(&json_path);
    let _ = fs::remove_file(&stderr_path);
    let segments = parse_whisper_json(&raw?)?;
    on_progress(100);
    Ok(segments)
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
        if text.starts_with('[')
            && let Some(end) = text.find(']')
            && text[..end].contains("-->")
        {
            text = text[end + 1..].trim();
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
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

fn read_log_tail(path: &Path) -> String {
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let start = file
        .metadata()
        .map(|metadata| metadata.len().saturating_sub(16 * 1024))
        .unwrap_or(0);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut tail = Vec::new();
    let _ = file.read_to_end(&mut tail);
    String::from_utf8_lossy(&tail).into_owned()
}

fn parse_progress_percentage(line: &str) -> Option<u8> {
    let (_, rest) = line.split_once("progress =")?;
    let percentage = rest.split_once('%')?.0.trim();
    percentage
        .parse::<u8>()
        .ok()
        .filter(|percentage| *percentage <= 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_english_model_from_preference_list() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("ggml-base.en.bin");
        fs::write(&model, []).unwrap();

        assert_eq!(detect_model_in_dir(dir.path()), Some(model));
    }

    #[test]
    fn detects_unknown_model_by_sorted_filename() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("ggml-foo.bin");
        fs::write(dir.path().join("ggml-zeta.bin"), []).unwrap();
        fs::write(&first, []).unwrap();

        assert_eq!(detect_model_in_dir(dir.path()), Some(first));
    }

    #[test]
    fn returns_none_for_empty_model_directory() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(detect_model_in_dir(dir.path()), None);
    }

    #[test]
    fn respects_model_preference_order() {
        let dir = tempfile::tempdir().unwrap();
        let preferred = dir.path().join("ggml-small.bin");
        fs::write(dir.path().join("ggml-base.bin"), []).unwrap();
        fs::write(dir.path().join("ggml-small.en.bin"), []).unwrap();
        fs::write(&preferred, []).unwrap();

        assert_eq!(detect_model_in_dir(dir.path()), Some(preferred));
    }

    #[test]
    fn parses_whisper_progress_percentages() {
        assert_eq!(
            parse_progress_percentage("whisper_print_progress: progress = 42%"),
            Some(42)
        );
        assert_eq!(parse_progress_percentage("progress = 100%"), Some(100));
        assert_eq!(parse_progress_percentage("progress = 101%"), None);
        assert_eq!(parse_progress_percentage("no progress here"), None);
    }
}
