use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Result, anyhow};

use super::segments::TranscriptSegment;

#[derive(Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentJob {
    pub session_id: String,
    pub generation: u64,
    pub segment_id: u64,
    pub audio_path: PathBuf,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    Queued,
    Full,
    Stale,
    Cancelled,
}

pub trait SegmentTranscriber {
    fn transcribe(&mut self, job: &SegmentJob) -> Result<String>;
}

pub struct IncrementalRunner<T> {
    session_id: String,
    generation: u64,
    max_queue: usize,
    pending: VecDeque<SegmentJob>,
    transcriber: T,
    cancel_token: CancelToken,
}

impl<T> IncrementalRunner<T> {
    pub fn new(session_id: impl Into<String>, max_queue: usize, transcriber: T) -> Self {
        Self {
            session_id: session_id.into(),
            generation: 0,
            max_queue,
            pending: VecDeque::new(),
            transcriber,
            cancel_token: CancelToken::new(),
        }
    }

    pub fn cancel_token(&self) -> CancelToken {
        self.cancel_token.clone()
    }

    pub fn new_job(
        &self,
        segment_id: u64,
        audio_path: impl Into<PathBuf>,
        start_ms: u64,
        end_ms: u64,
    ) -> SegmentJob {
        SegmentJob {
            session_id: self.session_id.clone(),
            generation: self.generation,
            segment_id,
            audio_path: audio_path.into(),
            start_ms,
            end_ms,
        }
    }

    pub fn reset(&mut self, session_id: impl Into<String>) {
        self.session_id = session_id.into();
        self.generation = self.generation.saturating_add(1);
        self.pending.clear();
        self.cancel_token = CancelToken::new();
    }

    pub fn enqueue(&mut self, job: SegmentJob) -> EnqueueResult {
        if self.cancel_token.is_cancelled() {
            return EnqueueResult::Cancelled;
        }
        if !self.is_current(&job) {
            return EnqueueResult::Stale;
        }
        if self.pending.len() >= self.max_queue {
            return EnqueueResult::Full;
        }
        self.pending.push_back(job);
        EnqueueResult::Queued
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn is_current(&self, job: &SegmentJob) -> bool {
        job.session_id == self.session_id && job.generation == self.generation
    }
}

impl<T: SegmentTranscriber> IncrementalRunner<T> {
    pub fn transcribe_next(&mut self) -> Option<TranscriptSegment> {
        if self.cancel_token.is_cancelled() {
            self.pending.clear();
            return None;
        }

        let job = self.pending.pop_front()?;
        let result = if self.is_current(&job) {
            self.transcriber.transcribe(&job)
        } else {
            Err(anyhow!("stale segment result"))
        };
        self.finish_job(&job, result)
    }

    pub fn finish_job(
        &self,
        job: &SegmentJob,
        result: Result<String>,
    ) -> Option<TranscriptSegment> {
        if self.cancel_token.is_cancelled() || !self.is_current(job) {
            return None;
        }

        Some(match result {
            Ok(text) => {
                TranscriptSegment::raw_ready(job.segment_id, job.start_ms, job.end_ms, text)
            }
            Err(err) => TranscriptSegment::failed(
                job.segment_id,
                job.start_ms,
                job.end_ms,
                format!("{err:#}"),
            ),
        })
    }
}

pub struct IncrementalSession {
    pub id: String,
    pub temp_dir: PathBuf,
    pub cancel_token: CancelToken,
}

impl IncrementalSession {
    pub fn new(id: impl Into<String>, temp_dir: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            temp_dir: temp_dir.into(),
            cancel_token: CancelToken::new(),
        }
    }

    pub fn cancel(&self, keep_audio: bool) -> Result<()> {
        self.cancel_token.cancel();
        cleanup_session_dir(&self.temp_dir, keep_audio)
    }
}

pub fn cleanup_session_dir(path: &Path, keep_audio: bool) -> Result<()> {
    if keep_audio || !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MockTranscriber {
        results: VecDeque<Result<String>>,
    }

    impl MockTranscriber {
        fn new(results: Vec<Result<String>>) -> Self {
            Self {
                results: results.into(),
            }
        }
    }

    impl SegmentTranscriber for MockTranscriber {
        fn transcribe(&mut self, _job: &SegmentJob) -> Result<String> {
            self.results
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pickscribe-{name}-{id}"))
    }

    #[test]
    fn runner_enforces_backpressure_cap() {
        let mut runner = IncrementalRunner::new("session", 1, MockTranscriber::new(vec![]));
        let first = runner.new_job(1, "one.wav", 0, 5_000);
        let second = runner.new_job(2, "two.wav", 5_000, 10_000);

        assert_eq!(runner.enqueue(first), EnqueueResult::Queued);
        assert_eq!(runner.enqueue(second), EnqueueResult::Full);
        assert_eq!(runner.pending_len(), 1);
    }

    #[test]
    fn runner_returns_raw_ready_and_failed_segments() {
        let mut runner = IncrementalRunner::new(
            "session",
            2,
            MockTranscriber::new(vec![Ok("hello".into()), Err(anyhow!("boom"))]),
        );
        let first = runner.new_job(1, "one.wav", 0, 5_000);
        let second = runner.new_job(2, "two.wav", 5_000, 10_000);
        runner.enqueue(first);
        runner.enqueue(second);

        let first = runner.transcribe_next().unwrap();
        let second = runner.transcribe_next().unwrap();

        assert_eq!(first.raw_text, "hello");
        assert_eq!(second.error.as_deref(), Some("boom"));
    }

    #[test]
    fn runner_ignores_stale_results_after_reset() {
        let mut runner = IncrementalRunner::new("session-a", 2, MockTranscriber::new(vec![]));
        let stale = runner.new_job(1, "one.wav", 0, 5_000);
        runner.reset("session-b");

        assert_eq!(runner.enqueue(stale.clone()), EnqueueResult::Stale);
        assert!(runner.finish_job(&stale, Ok("late text".into())).is_none());
    }

    #[test]
    fn runner_cancel_clears_pending_and_ignores_results() {
        let mut runner = IncrementalRunner::new(
            "session",
            2,
            MockTranscriber::new(vec![Ok("ignored".into())]),
        );
        let job = runner.new_job(1, "one.wav", 0, 5_000);
        runner.enqueue(job.clone());
        runner.cancel_token().cancel();

        assert!(runner.transcribe_next().is_none());
        assert_eq!(runner.pending_len(), 0);
        assert!(runner.finish_job(&job, Ok("late".into())).is_none());
    }

    #[test]
    fn session_cancel_removes_temp_dir_unless_keep_audio() -> Result<()> {
        let remove_dir = temp_dir("incremental-remove");
        fs::create_dir_all(&remove_dir)?;
        fs::write(remove_dir.join("segment.wav"), b"data")?;
        IncrementalSession::new("remove", &remove_dir).cancel(false)?;
        assert!(!remove_dir.exists());

        let keep_dir = temp_dir("incremental-keep");
        fs::create_dir_all(&keep_dir)?;
        fs::write(keep_dir.join("segment.wav"), b"data")?;
        IncrementalSession::new("keep", &keep_dir).cancel(true)?;
        assert!(keep_dir.exists());
        fs::remove_dir_all(keep_dir).ok();
        Ok(())
    }
}
