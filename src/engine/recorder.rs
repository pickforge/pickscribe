use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::config::SttConfig;

pub struct Recording {
    child: Option<Child>,
    pub audio_path: PathBuf,
    pub log_path: PathBuf,
    pub started: Instant,
}

pub fn state_dir() -> PathBuf {
    state_dir_from_env(|name| std::env::var(name).ok())
}

fn state_dir_from_env(mut var: impl FnMut(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = var("PICKSCRIBE_STATE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Some(dir) = var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("pickscribe");
    }
    if let Some(dir) = var("TMPDIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("pickscribe");
    }
    let user = var("USER").unwrap_or_else(|| "user".into());
    PathBuf::from(format!("/tmp/pickscribe-{user}"))
}

pub fn prepare_state_dir() -> Result<PathBuf> {
    let dir = state_dir();
    prepare_state_dir_at(&dir)?;
    Ok(dir)
}

fn prepare_state_dir_at(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("securing {}", dir.display()))?;
    }
    Ok(())
}

pub fn start(cfg: &SttConfig) -> Result<Recording> {
    let dir = prepare_state_dir()?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let audio_path = dir.join(format!("recording-{stamp}.wav"));
    let log_path = dir.join(format!("recording-{stamp}.log"));
    let log_file = fs::File::create(&log_path).context("creating recorder log file")?;

    let recorder = if cfg.recorder.is_empty() {
        crate::config::default_recorder_command()
    } else {
        &cfg.recorder
    };
    let program = if recorder.contains('/') {
        PathBuf::from(recorder)
    } else {
        super::find_command(recorder).unwrap_or_else(|| PathBuf::from(recorder))
    };
    let args = recorder_args(recorder, cfg, &audio_path);
    let mut cmd = Command::new(program);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file));

    let child = cmd
        .spawn()
        .with_context(|| format!("starting recorder `{recorder}`"))?;

    Ok(Recording {
        child: Some(child),
        audio_path,
        log_path,
        started: Instant::now(),
    })
}

/// Build the recorder's argument vector for the given recorder command,
/// config, and output path. The ffmpeg/avfoundation shape is selected purely
/// by the recorder command's file stem, matching the pw-record vs. ffmpeg
/// binaries the recorder can actually be pointed at.
fn recorder_args(recorder: &str, cfg: &SttConfig, audio_path: &Path) -> Vec<String> {
    let stem = std::path::Path::new(recorder)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(recorder);

    if stem == "ffmpeg" {
        let target = if !cfg.audio_target.is_empty() {
            cfg.audio_target.clone()
        } else {
            "default".into()
        };
        return vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-f".into(),
            "avfoundation".into(),
            "-i".into(),
            format!(":{target}"),
            "-ar".into(),
            "16000".into(),
            "-ac".into(),
            "1".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            "-y".into(),
            audio_path.display().to_string(),
        ];
    }

    let mut args = vec![
        "--media-category".to_string(),
        "Capture".into(),
        "--media-role".into(),
        "Communication".into(),
        "--rate".into(),
        "16000".into(),
        "--channels".into(),
        "1".into(),
        "--format".into(),
        "s16".into(),
    ];
    if !cfg.audio_target.is_empty() {
        args.push("--target".into());
        args.push(cfg.audio_target.clone());
    }
    args.push(audio_path.display().to_string());
    args
}

impl Recording {
    pub fn duration_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// If the recorder process already exited (e.g. bad device, missing
    /// binary flags), return a description of the failure. Callers poll this
    /// shortly after `start` — off the UI thread — instead of `start`
    /// sleeping on the command path.
    pub fn exit_error(&mut self) -> Option<String> {
        let status = self.child.as_mut()?.try_wait().ok()??;
        let log = fs::read_to_string(&self.log_path).unwrap_or_default();
        Some(format!(
            "recorder exited immediately ({status}): {}",
            log.trim()
        ))
    }

    /// Stop the recorder gracefully (SIGINT so the WAV header is finalized),
    /// escalating to SIGTERM/SIGKILL if needed.
    pub fn stop(mut self) -> Result<(PathBuf, u64)> {
        let duration = self.duration_ms();
        let mut child = self
            .child
            .take()
            .context("recorder process already stopped")?;
        let pid = child.id();

        signal(pid, "INT");
        if !wait_exit(&mut child, Duration::from_secs(5)) {
            signal(pid, "TERM");
            if !wait_exit(&mut child, Duration::from_secs(2)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        let meta = fs::metadata(&self.audio_path)
            .with_context(|| format!("recording file missing: {}", self.audio_path.display()))?;
        if meta.len() < 8 * 1024 {
            let _ = fs::remove_file(&self.audio_path);
            let _ = fs::remove_file(&self.log_path);
            bail!("recording too short — no audio captured");
        }
        let _ = fs::remove_file(&self.log_path);
        Ok((self.audio_path.clone(), duration))
    }

    /// Stop and discard everything.
    pub fn cancel(mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            signal(pid, "INT");
            if !wait_exit(&mut child, Duration::from_secs(2)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        let _ = fs::remove_file(&self.audio_path);
        let _ = fs::remove_file(&self.log_path);
    }

    #[cfg(test)]
    fn from_child(child: Child) -> Self {
        Self {
            child: Some(child),
            audio_path: PathBuf::new(),
            log_path: PathBuf::new(),
            started: Instant::now(),
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                signal(child.id(), "INT");
                if !wait_exit(&mut child, Duration::from_secs(2)) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
}

fn signal(pid: u32, sig: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{sig}"))
        .arg(pid.to_string())
        .status();
}

fn wait_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_dir_uses_tmpdir_before_the_shared_tmp_fallback() {
        let dir = state_dir_from_env(|name| match name {
            "TMPDIR" => Some("/private/var/folders/user/T".into()),
            "USER" => Some("alice".into()),
            _ => None,
        });

        assert_eq!(dir, PathBuf::from("/private/var/folders/user/T/pickscribe"));
    }

    #[test]
    fn state_dir_keeps_explicit_and_xdg_precedence() {
        let explicit = state_dir_from_env(|name| match name {
            "PICKSCRIBE_STATE_DIR" => Some("/custom/state".into()),
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            "TMPDIR" => Some("/private/tmp".into()),
            _ => None,
        });
        let xdg = state_dir_from_env(|name| match name {
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            "TMPDIR" => Some("/private/tmp".into()),
            _ => None,
        });

        assert_eq!(explicit, PathBuf::from("/custom/state"));
        assert_eq!(xdg, PathBuf::from("/run/user/1000/pickscribe"));
    }

    #[cfg(unix)]
    #[test]
    fn prepared_state_dir_has_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("state");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        prepare_state_dir_at(&dir).unwrap();

        let mode = fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_recording_terminates_and_reaps_child() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();

        drop(Recording::from_child(child));

        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "recording child {pid} is still alive");
    }

    #[test]
    fn pw_record_args_are_unchanged() {
        let cfg = SttConfig {
            recorder: "pw-record".into(),
            ..Default::default()
        };
        let audio_path = PathBuf::from("/tmp/rec.wav");

        let args = recorder_args("pw-record", &cfg, &audio_path);

        assert_eq!(
            args,
            vec![
                "--media-category",
                "Capture",
                "--media-role",
                "Communication",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "s16",
                "/tmp/rec.wav",
            ]
        );
    }

    #[test]
    fn pw_record_args_include_target_when_set() {
        let cfg = SttConfig {
            recorder: "pw-record".into(),
            audio_target: "alsa_input.usb-mic".into(),
            ..Default::default()
        };
        let audio_path = PathBuf::from("/tmp/rec.wav");

        let args = recorder_args("pw-record", &cfg, &audio_path);

        assert!(args.contains(&"--target".to_string()));
        assert!(args.contains(&"alsa_input.usb-mic".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/rec.wav");
    }

    #[test]
    fn ffmpeg_args_use_default_avfoundation_target_on_macos() {
        let cfg = SttConfig {
            recorder: "ffmpeg".into(),
            ..Default::default()
        };
        let audio_path = PathBuf::from("/tmp/rec.wav");

        let args = recorder_args("ffmpeg", &cfg, &audio_path);

        assert_eq!(
            args,
            vec![
                "-nostdin",
                "-hide_banner",
                "-f",
                "avfoundation",
                "-i",
                ":default",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-c:a",
                "pcm_s16le",
                "-y",
                "/tmp/rec.wav",
            ]
        );
    }

    #[test]
    fn ffmpeg_args_use_explicit_audio_target() {
        let cfg = SttConfig {
            recorder: "ffmpeg".into(),
            audio_target: "2".into(),
            ..Default::default()
        };
        let audio_path = PathBuf::from("/tmp/rec.wav");

        let args = recorder_args("ffmpeg", &cfg, &audio_path);

        assert!(args.contains(&":2".to_string()));
    }

    #[test]
    fn ffmpeg_selection_is_driven_by_recorder_stem_not_os() {
        let cfg = SttConfig {
            recorder: "/opt/homebrew/bin/ffmpeg".into(),
            ..Default::default()
        };
        let audio_path = PathBuf::from("/tmp/rec.wav");

        let args = recorder_args("/opt/homebrew/bin/ffmpeg", &cfg, &audio_path);

        assert!(args.contains(&"avfoundation".to_string()));
    }
}
