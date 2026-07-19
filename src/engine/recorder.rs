use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::config::SttConfig;

pub struct Recording {
    child: Child,
    pub audio_path: PathBuf,
    pub log_path: PathBuf,
    pub started: Instant,
}

pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PICKSCRIBE_STATE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("pickscribe");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    PathBuf::from(format!("/tmp/pickscribe-{user}"))
}

pub fn start(cfg: &SttConfig) -> Result<Recording> {
    let dir = state_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let audio_path = dir.join(format!("recording-{stamp}.wav"));
    let log_path = dir.join(format!("recording-{stamp}.log"));
    let log_file = fs::File::create(&log_path).context("creating recorder log file")?;

    let recorder = if cfg.recorder.is_empty() {
        "pw-record"
    } else {
        &cfg.recorder
    };
    let program = if recorder.contains('/') {
        PathBuf::from(recorder)
    } else {
        super::find_command(recorder).unwrap_or_else(|| PathBuf::from(recorder))
    };
    let mut cmd = Command::new(program);
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
    if !cfg.audio_target.is_empty() {
        cmd.arg("--target").arg(&cfg.audio_target);
    }
    cmd.arg(&audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file));

    let child = cmd
        .spawn()
        .with_context(|| format!("starting recorder `{recorder}`"))?;

    Ok(Recording {
        child,
        audio_path,
        log_path,
        started: Instant::now(),
    })
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
        let status = self.child.try_wait().ok()??;
        let log = fs::read_to_string(&self.log_path).unwrap_or_default();
        Some(format!(
            "recorder exited immediately ({status}): {}",
            log.trim()
        ))
    }

    /// Stop the recorder gracefully (SIGINT so the WAV header is finalized),
    /// escalating to SIGTERM/SIGKILL if needed.
    pub fn stop(mut self) -> Result<(PathBuf, u64)> {
        let pid = self.child.id();
        let duration = self.duration_ms();

        signal(pid, "INT");
        if !wait_exit(&mut self.child, Duration::from_secs(5)) {
            signal(pid, "TERM");
            if !wait_exit(&mut self.child, Duration::from_secs(2)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
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
        let pid = self.child.id();
        signal(pid, "INT");
        if !wait_exit(&mut self.child, Duration::from_secs(2)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_file(&self.audio_path);
        let _ = fs::remove_file(&self.log_path);
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
