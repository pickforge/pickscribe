use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use pickscribe::config::AppConfig;
use pickscribe::engine::{
    cleanup,
    incremental::CancelToken,
    media::{self, MEDIA_EXTENSIONS},
    recorder, stt, transcript,
};
use pickscribe::history::NewEntry;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::engine::{EVENT_HISTORY, Engine};

pub const EVENT_FILE: &str = "pickscribe://file";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStage {
    Converting,
    Transcribing,
    Cleaning,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileJobState {
    pub stage: FileStage,
    pub progress: u8,
    pub source_file: String,
    pub error: Option<String>,
    pub entry_id: Option<i64>,
}

pub(crate) struct FileJobControl {
    pub(crate) cancel_token: CancelToken,
}

pub fn start(
    engine: Arc<Engine>,
    app: AppHandle,
    source_file: String,
    cleanup_requested: bool,
) -> Result<()> {
    validate_input_path(Path::new(&source_file))?;

    let cancel_token = CancelToken::new();
    {
        let mut file_job = engine.file_job.lock().unwrap();
        if file_job.is_some() {
            bail!("a file transcription is already running");
        }
        *file_job = Some(FileJobControl {
            cancel_token: cancel_token.clone(),
        });
    }

    std::thread::spawn(move || {
        run_file_job(engine, app, source_file, cleanup_requested, cancel_token)
    });
    Ok(())
}

struct FileJobGuard {
    engine: Arc<Engine>,
    temp_dir: Option<PathBuf>,
}

impl FileJobGuard {
    fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            temp_dir: None,
        }
    }

    fn set_temp_dir(&mut self, temp_dir: PathBuf) {
        self.temp_dir = Some(temp_dir);
    }
}

impl Drop for FileJobGuard {
    fn drop(&mut self) {
        if let Some(temp_dir) = self.temp_dir.take() {
            let _ = fs::remove_dir_all(temp_dir);
        }
        match self.engine.file_job.lock() {
            Ok(mut file_job) => *file_job = None,
            Err(poisoned) => {
                *poisoned.into_inner() = None;
                self.engine.file_job.clear_poison();
            }
        }
    }
}

pub fn cancel(engine: &Engine) {
    if let Some(file_job) = engine.file_job.lock().unwrap().as_ref() {
        file_job.cancel_token.cancel();
    }
}

fn validate_input_path(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("media file not found: {}", path.display());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .context("media file has no extension")?;
    if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        bail!("unsupported media file extension: .{extension}");
    }
    Ok(())
}

// TODO(#63): split legacy file-job orchestration into capped helpers.
#[allow(clippy::too_many_lines)]
fn run_file_job(
    engine: Arc<Engine>,
    app: AppHandle,
    source_file: String,
    cleanup_requested: bool,
    cancel_token: CancelToken,
) {
    let progress = Arc::new(AtomicU8::new(0));
    let mut guard = FileJobGuard::new(Arc::clone(&engine));
    let result = (|| -> Result<FileJobComplete> {
        let dir = create_temp_dir()?;
        guard.set_temp_dir(dir.clone());
        let wav = dir.join("audio.wav");
        let cfg = AppConfig::load();

        if cancel_token.is_cancelled() {
            bail!("file transcription cancelled");
        }
        emit_state(&app, FileStage::Converting, 0, &source_file, None, None);
        media::convert_to_wav_16k_mono(Path::new(&source_file), &wav, || {
            cancel_token.is_cancelled()
        })?;
        if cancel_token.is_cancelled() {
            bail!("file transcription cancelled");
        }

        let duration_ms = media::wav_duration_ms(&wav)?;
        emit_state(&app, FileStage::Transcribing, 0, &source_file, None, None);
        let progress_for_callback = Arc::clone(&progress);
        let app_for_callback = app.clone();
        let source_for_callback = source_file.clone();
        let segments = stt::transcribe_file_with_cancel(
            &cfg.stt,
            &wav,
            || cancel_token.is_cancelled(),
            move |percentage| {
                if progress_for_callback.swap(percentage, Ordering::Relaxed) != percentage {
                    emit_state(
                        &app_for_callback,
                        FileStage::Transcribing,
                        percentage,
                        &source_for_callback,
                        None,
                        None,
                    );
                }
            },
        )?;
        if cancel_token.is_cancelled() {
            bail!("file transcription cancelled");
        }

        let raw_text = transcript::to_txt(&segments);
        if raw_text.is_empty() {
            bail!("no speech detected in the file");
        }
        let (cleaned_text, provider, model, cleanup_error) = if cleanup_requested {
            emit_state(
                &app,
                FileStage::Cleaning,
                progress.load(Ordering::Relaxed),
                &source_file,
                None,
                None,
            );
            let outcome = cleanup::clean(&cfg, &raw_text);
            if cancel_token.is_cancelled() {
                bail!("file transcription cancelled");
            }
            (
                (outcome.text != raw_text).then_some(outcome.text),
                outcome.provider,
                outcome.model,
                outcome
                    .error
                    .map(|error| format!("cleanup skipped: {error}")),
            )
        } else {
            (None, "none".to_string(), String::new(), None)
        };
        let segments_json = if segments.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&segments).context("serializing file segments")?)
        };
        if cancel_token.is_cancelled() {
            bail!("file transcription cancelled");
        }

        let entry_id = engine.history.insert(&NewEntry {
            duration_ms,
            raw_text,
            cleaned_text,
            provider,
            model,
            language: cfg.stt.language,
            source_file: Some(source_file.clone()),
            segments_json,
        })?;
        let _ = app.emit(EVENT_HISTORY, ());
        Ok(FileJobComplete {
            entry_id,
            cleanup_error,
        })
    })();

    match result {
        Ok(result) => emit_state(
            &app,
            FileStage::Done,
            progress.load(Ordering::Relaxed),
            &source_file,
            result.cleanup_error,
            Some(result.entry_id),
        ),
        Err(_) if cancel_token.is_cancelled() => emit_state(
            &app,
            FileStage::Cancelled,
            progress.load(Ordering::Relaxed),
            &source_file,
            None,
            None,
        ),
        Err(err) => emit_state(
            &app,
            FileStage::Error,
            progress.load(Ordering::Relaxed),
            &source_file,
            Some(format!("{err:#}")),
            None,
        ),
    }
}

struct FileJobComplete {
    entry_id: i64,
    cleanup_error: Option<String>,
}

fn create_temp_dir() -> Result<PathBuf> {
    let parent = recorder::state_dir();
    fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = parent.join(format!("file-{}-{stamp}", std::process::id()));
    fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn emit_state(
    app: &AppHandle,
    stage: FileStage,
    progress: u8,
    source_file: &str,
    error: Option<String>,
    entry_id: Option<i64>,
) {
    let _ = app.emit(
        EVENT_FILE,
        FileJobState {
            stage,
            progress,
            source_file: source_file.to_string(),
            error,
            entry_id,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> Result<PathBuf> {
        // A timestamp alone can collide when parallel test threads hit the
        // same clock tick; the atomic counter makes each dir unique.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "pickscribe-file-job-{}-{stamp}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&dir)?;
        Ok(dir)
    }

    #[test]
    fn validates_supported_media_extensions_case_insensitively() -> Result<()> {
        let dir = temp_dir()?;
        let path = dir.join("recording.MP4");
        fs::write(&path, [])?;

        validate_input_path(&path)?;

        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn rejects_unsupported_media_extensions() -> Result<()> {
        let dir = temp_dir()?;
        let path = dir.join("recording.txt");
        fs::write(&path, [])?;

        let error = validate_input_path(&path).unwrap_err();

        assert!(error.to_string().contains("unsupported media file extension"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
