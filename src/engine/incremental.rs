//! Deep module for incremental transcription orchestration.
//!
//! `src/bin/pickscribe.rs` (a separate worker process) and
//! `src-tauri/src/engine.rs` (a background thread) each own an incremental
//! *adapter*: they grow a WAV file while recording, and want finalized
//! chunks transcribed (and optionally cleaned) as they go, so stopping
//! doesn't pay for one long final STT pass. Everything about *when* to cut
//! a segment, when the live backlog or the final drain must give up and
//! fall back to a full re-transcription, and how progress accumulates was,
//! until this module, duplicated almost line-for-line in both adapters —
//! including one live divergence (see [`SchedulingConfig`] docs below).
//!
//! This module owns that policy as a single deterministic state machine
//! (see [`next_step`], [`after_boundary`], [`classify_slice`] and the
//! driver, [`run`]). STT execution and live segment cleanup remain
//! adapters at the seam — implemented once per binary via the
//! [`IncrementalHost`] trait — because they are genuinely different
//! mechanisms (a subprocess with file-based cancellation polling vs. an
//! in-process call gated by a session token), not scheduling policy.
//! Progress publication (write a JSON file vs. emit a Tauri event) is the
//! other adapter seam; the *decision* of when progress changed enough to
//! publish lives in `run`, not in either adapter.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::audio_segments::{self, WavSlice};
use super::segments::{RecordingSession, TranscriptSegment, TranscriptSegmentStatus};

/// How long the scheduler waits before re-checking a growing recording when
/// there isn't yet enough (or any) new audio to act on.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Session-scoped cooperative cancellation flag. Shared by incremental
/// scheduling and other session-lifetime work (e.g. file-job cancellation)
/// that needs the same cheap, cloneable on/off switch.
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

/// Remove an incremental session's scratch directory (segment WAVs, worker
/// state) unless the caller opted to keep recorded audio. Shared by every
/// caller that tears down an incremental session: normal completion,
/// cancellation, and abandonment.
pub fn cleanup_session_dir(path: &Path, keep_audio: bool) -> Result<()> {
    if keep_audio || !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(Into::into)
}

/// Normalized incremental scheduling knobs. Both adapters build this from
/// `IncrementalConfig` via [`SchedulingConfig::new`], which is the single
/// place the target/max/overlap/backlog clamps are applied — previously
/// duplicated in both worker loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulingConfig {
    pub target_ms: u64,
    pub max_ms: u64,
    pub overlap_ms: u64,
    pub backlog_limit_ms: u64,
}

impl SchedulingConfig {
    pub fn new(target_ms: u64, max_ms: u64, overlap_ms: u64, max_queue: usize) -> Self {
        let target_ms = target_ms.max(1_000);
        let max_ms = max_ms.max(target_ms);
        let overlap_ms = overlap_ms.min(target_ms / 2);
        let backlog_limit_ms = max_ms.saturating_mul(max_queue.max(1) as u64);
        Self {
            target_ms,
            max_ms,
            overlap_ms,
            backlog_limit_ms,
        }
    }
}

/// Terminal decision for the worker loop: whether the incremental result is
/// usable as-is, or a fallback (full/partial re-transcription) is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopOutcome {
    pub complete: bool,
    pub fallback_required: bool,
}

impl StopOutcome {
    /// No more audio can be produced into a new segment (a boundary/slice
    /// couldn't advance past `next_start_ms`) while a final drain was
    /// requested. Complete only if every buffered sample was already
    /// consumed by a finished segment; otherwise there is unconsumed audio
    /// with nothing to show for it, so a fallback is required.
    fn stuck(next_start_ms: u64, available_ms: u64) -> Self {
        let fallback_required = available_ms > next_start_ms;
        Self {
            complete: !fallback_required,
            fallback_required,
        }
    }

    fn fallback() -> Self {
        Self {
            complete: false,
            fallback_required: true,
        }
    }

    fn finished() -> Self {
        Self {
            complete: true,
            fallback_required: false,
        }
    }

    fn cancelled() -> Self {
        Self {
            complete: false,
            fallback_required: false,
        }
    }
}

/// What the scheduler wants to do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not enough new audio yet; sleep and re-check.
    Wait,
    /// Cut a segment starting at `start_ms`, targeting `desired_end_ms`
    /// (the caller still refines this to a low-energy boundary).
    Produce { start_ms: u64, desired_end_ms: u64 },
    /// Stop the loop with this terminal outcome.
    Stop(StopOutcome),
}

/// The core scheduling decision, given how much audio is available and
/// whether a final drain was requested. Pure function of the inputs — no
/// I/O, no adapter state — so both worker loops now share one policy
/// instead of two copies that could (and did) diverge.
///
/// **Live backlog overflow policy (unified):** when the buffered-but-not-
/// yet-segmented audio exceeds `backlog_limit_ms` *before* a final drain is
/// requested, this stops the loop immediately with a fallback outcome
/// rather than continuing to produce segments that a fallback will discard
/// anyway. Before this module, the CLI worker already did this; the
/// desktop worker instead set `fallback_required` and kept scheduling
/// segments until the final drain or cancellation. The CLI's stop-
/// immediately behavior wins here: it stops burning CPU/STT time on
/// segments a fallback will discard, and it maximizes the contiguous
/// finished prefix available to `segments::salvage_completed_prefix`
/// (PR #49) by not letting more segments race the fallback decision.
///
/// **Final backlog overflow** (buffered audio exceeds `max_ms` once a
/// final drain was requested) always stopped both loops immediately
/// already; that policy is unchanged, just centralized here.
pub fn next_step(
    next_start_ms: u64,
    available_ms: u64,
    final_requested: bool,
    cfg: &SchedulingConfig,
) -> Step {
    // 250ms residual tail: audio shorter than that past the last segment
    // boundary is never worth cutting into its own segment.
    if available_ms <= next_start_ms.saturating_add(250) {
        return if final_requested {
            Step::Stop(StopOutcome::stuck(next_start_ms, available_ms))
        } else {
            Step::Wait
        };
    }

    let buffered_ms = available_ms.saturating_sub(next_start_ms);
    if !final_requested && buffered_ms < cfg.target_ms {
        return Step::Wait;
    }
    if !final_requested && buffered_ms > cfg.backlog_limit_ms {
        return Step::Stop(StopOutcome::fallback());
    }
    if final_requested && buffered_ms > cfg.max_ms {
        return Step::Stop(StopOutcome::fallback());
    }

    let desired_end_ms = if final_requested {
        next_start_ms.saturating_add(cfg.max_ms).min(available_ms)
    } else {
        next_start_ms.saturating_add(cfg.target_ms).min(available_ms)
    };
    Step::Produce {
        start_ms: next_start_ms,
        desired_end_ms,
    }
}

/// Refine a desired segment end to a nearby low-energy (silence) boundary
/// so segments don't cut mid-word. Reads a small window of samples around
/// `desired_end_ms`; on any read failure it degrades to the desired end.
pub fn choose_segment_end(
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

    let target_sample =
        audio_segments::ms_to_sample(desired_end_ms.saturating_sub(scan_start_ms)) as usize;
    let radius_sample = (audio_segments::ms_to_sample(radius_ms) as usize).min(samples.len());
    let boundary_sample =
        audio_segments::find_low_energy_boundary(&samples, target_sample, radius_sample);
    let boundary_ms =
        scan_start_ms.saturating_add(audio_segments::sample_to_ms(boundary_sample as u64));
    boundary_ms.clamp(next_start_ms.saturating_add(250), available_ms)
}

/// Whether the refined boundary actually advances past `next_start_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryStep {
    Wait,
    Stop(StopOutcome),
    Proceed,
}

pub fn after_boundary(
    next_start_ms: u64,
    end_ms: u64,
    available_ms: u64,
    final_requested: bool,
) -> BoundaryStep {
    if end_ms > next_start_ms {
        return BoundaryStep::Proceed;
    }
    if final_requested {
        BoundaryStep::Stop(StopOutcome::stuck(next_start_ms, available_ms))
    } else {
        BoundaryStep::Wait
    }
}

/// What to do with the result of slicing a segment's WAV out of the
/// growing recording.
pub enum SliceStep {
    Use(WavSlice),
    Wait,
    Stop(StopOutcome),
    /// Slicing failed while a final drain was requested: the caller should
    /// record a failed segment (its audio range has no text) and fall back.
    Failed(String),
}

pub fn classify_slice(
    result: Result<WavSlice>,
    final_requested: bool,
    available_ms: u64,
    next_start_ms: u64,
) -> SliceStep {
    match result {
        Ok(slice) if slice.sample_count > 0 => SliceStep::Use(slice),
        Ok(_) if final_requested => SliceStep::Stop(StopOutcome::stuck(next_start_ms, available_ms)),
        Ok(_) => SliceStep::Wait,
        Err(err) if final_requested => SliceStep::Failed(format!("{err:#}")),
        Err(_) => SliceStep::Wait,
    }
}

/// One segment's transcription work, handed to [`IncrementalHost::transcribe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentJob {
    pub segment_id: u64,
    pub audio_path: PathBuf,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// What the scheduler should do right now, from the host's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Control {
    #[default]
    Continue,
    /// A final drain was requested (recording stopped); keep going until
    /// buffered audio is exhausted or a fallback condition is hit.
    Stopping,
    /// Stop now without publishing a terminal result — the caller
    /// (cancellation) doesn't want the in-progress work.
    Cancelled,
    /// The owning process/session is gone though no stop was requested
    /// (e.g. the CLI's parent process died before asking to stop). Stop
    /// now, clean up scratch artifacts, and do not publish anything —
    /// there is nothing left to read the result.
    Abandoned,
}

/// The adapter seam: everything specific to running as a CLI worker
/// subprocess vs. a desktop background thread. Scheduling policy in `run`
/// never touches recorder/session mechanics directly — only through these
/// methods.
pub trait IncrementalHost {
    /// Poll whatever this host uses to represent stop/cancel/abandon.
    fn control(&self) -> Control;
    fn keep_audio(&self) -> bool;
    /// Where to write a segment's sliced WAV before transcribing it.
    fn segment_path(&self, segment_id: u64) -> PathBuf;
    /// Transcribe one segment. Implementations poll their own cancellation
    /// source (subprocess file polling, session token) during the call;
    /// `run` re-checks `control()` immediately after regardless.
    fn transcribe(&mut self, job: &SegmentJob) -> Result<String>;
    /// Offer a finished segment to live cleanup (if enabled). Returns
    /// whether it was accepted (queued); the scheduler marks the segment
    /// `Cleaning` only when this returns `true`.
    fn try_queue_cleanup(&mut self, segment: TranscriptSegment) -> bool;
    /// Drain any cleanup results that arrived since the last call.
    fn drain_cleanup(&mut self) -> Vec<TranscriptSegment>;
    /// Publish the current segment list (Tauri event / worker output file).
    fn publish(&mut self, session: &RecordingSession);
    /// Called only on [`Control::Abandoned`], before `run` returns.
    fn cleanup_artifacts(&mut self);
}

/// The incremental worker's terminal result.
pub struct Outcome {
    pub session: RecordingSession,
    pub complete: bool,
    pub fallback_required: bool,
}

pub enum RunResult {
    Finished(Outcome),
    /// The host was abandoned; nothing was published and the caller should
    /// return without further processing (mirrors the CLI worker exiting
    /// when its parent process died).
    Abandoned,
}

/// Drive one incremental session to completion. Owns segment scheduling
/// (`next_step`/`choose_segment_end`/`after_boundary`/`classify_slice`),
/// progress state (the `RecordingSession` and when to publish it), the
/// fallback/completion decision, and the final drain. STT execution, live
/// segment cleanup, and progress transport are the host's job.
// TODO(#63): split the legacy session driver into capped orchestration helpers.
#[allow(clippy::too_many_lines)]
pub fn run(
    host: &mut impl IncrementalHost,
    audio_path: &Path,
    session_id: impl Into<String>,
    cfg: SchedulingConfig,
) -> RunResult {
    let mut session = RecordingSession::new(session_id);
    let mut next_start_ms = 0u64;
    let mut segment_id = 0u64;

    let stop = 'run: loop {
        if drain_and_publish(host, &mut session) {
            // published below alongside other state changes too; draining
            // alone is enough reason to republish.
        }

        match host.control() {
            Control::Abandoned => {
                host.cleanup_artifacts();
                return RunResult::Abandoned;
            }
            Control::Cancelled => break 'run StopOutcome::cancelled(),
            control => {
                let final_requested = matches!(control, Control::Stopping);
                let available_ms = audio_segments::duration_ms(audio_path).unwrap_or(0);

                let (start_ms, desired_end_ms) =
                    match next_step(next_start_ms, available_ms, final_requested, &cfg) {
                        Step::Wait => {
                            std::thread::sleep(POLL_INTERVAL);
                            continue 'run;
                        }
                        Step::Stop(stop) => break 'run stop,
                        Step::Produce {
                            start_ms,
                            desired_end_ms,
                        } => (start_ms, desired_end_ms),
                    };

                let end_ms =
                    choose_segment_end(audio_path, start_ms, desired_end_ms, available_ms, final_requested);
                match after_boundary(start_ms, end_ms, available_ms, final_requested) {
                    BoundaryStep::Wait => {
                        std::thread::sleep(POLL_INTERVAL);
                        continue 'run;
                    }
                    BoundaryStep::Stop(stop) => break 'run stop,
                    BoundaryStep::Proceed => {}
                }

                segment_id = segment_id.saturating_add(1);
                let slice_start_ms = start_ms.saturating_sub(cfg.overlap_ms);
                let segment_path = host.segment_path(segment_id);
                let slice_result =
                    audio_segments::slice_wav(audio_path, &segment_path, slice_start_ms, end_ms);

                let slice = match classify_slice(slice_result, final_requested, available_ms, start_ms) {
                    SliceStep::Wait => {
                        std::thread::sleep(POLL_INTERVAL);
                        continue 'run;
                    }
                    SliceStep::Stop(stop) => break 'run stop,
                    SliceStep::Failed(err) => {
                        session.upsert_segment(TranscriptSegment::failed(
                            segment_id,
                            slice_start_ms,
                            end_ms,
                            err,
                        ));
                        host.publish(&session);
                        break 'run StopOutcome::fallback();
                    }
                    SliceStep::Use(slice) => slice,
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
                host.publish(&session);

                let job = SegmentJob {
                    segment_id,
                    audio_path: segment_path.clone(),
                    start_ms: slice.start_ms,
                    end_ms: slice.end_ms,
                };
                let result = host.transcribe(&job);
                if !host.keep_audio() {
                    let _ = fs::remove_file(&segment_path);
                }

                match host.control() {
                    Control::Abandoned => {
                        host.cleanup_artifacts();
                        return RunResult::Abandoned;
                    }
                    Control::Cancelled => break 'run StopOutcome::cancelled(),
                    _ => {}
                }

                match result {
                    Ok(text) => {
                        let raw = TranscriptSegment::raw_ready(
                            segment_id,
                            slice.start_ms,
                            slice.end_ms,
                            text,
                        );
                        session.upsert_segment(raw.clone());
                        host.publish(&session);

                        if !raw.raw_text.trim().is_empty() && host.try_queue_cleanup(raw.clone()) {
                            session.upsert_segment(TranscriptSegment {
                                status: TranscriptSegmentStatus::Cleaning,
                                ..raw
                            });
                            host.publish(&session);
                        }
                    }
                    Err(err) => {
                        session.upsert_segment(TranscriptSegment::failed(
                            segment_id,
                            slice.start_ms,
                            slice.end_ms,
                            format!("{err:#}"),
                        ));
                        host.publish(&session);
                        break 'run StopOutcome::fallback();
                    }
                }

                next_start_ms = slice.end_ms;
                if final_requested && next_start_ms >= available_ms {
                    break 'run StopOutcome::finished();
                }
            }
        }
    };

    drain_and_publish(host, &mut session);

    RunResult::Finished(Outcome {
        session,
        complete: stop.complete,
        fallback_required: stop.fallback_required,
    })
}

fn drain_and_publish(host: &mut impl IncrementalHost, session: &mut RecordingSession) -> bool {
    let drained = host.drain_cleanup();
    if drained.is_empty() {
        return false;
    }
    for segment in drained {
        session.upsert_segment(segment);
    }
    host.publish(session);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque as Deque;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn cfg() -> SchedulingConfig {
        SchedulingConfig::new(5_000, 10_000, 1_500, 2)
    }

    // --- SchedulingConfig clamps --------------------------------------

    #[test]
    fn scheduling_config_enforces_minimum_target_and_half_target_overlap_cap() {
        let cfg = SchedulingConfig::new(200, 500, 5_000, 2);
        assert_eq!(cfg.target_ms, 1_000, "target floors at 1000ms");
        assert_eq!(cfg.max_ms, 1_000, "max floors up to the clamped target");
        assert_eq!(cfg.overlap_ms, 500, "overlap caps at half the clamped target");
    }

    #[test]
    fn scheduling_config_computes_backlog_limit_from_max_and_queue() {
        let cfg = SchedulingConfig::new(5_000, 10_000, 1_500, 3);
        assert_eq!(cfg.backlog_limit_ms, 30_000);
        let cfg_zero_queue = SchedulingConfig::new(5_000, 10_000, 1_500, 0);
        assert_eq!(
            cfg_zero_queue.backlog_limit_ms, 10_000,
            "max_queue floors at 1"
        );
    }

    // --- next_step: residual tail / stop-before-first-segment ----------

    #[test]
    fn stop_before_first_segment_is_complete_with_no_audio() {
        let step = next_step(0, 0, true, &cfg());
        assert_eq!(step, Step::Stop(StopOutcome { complete: true, fallback_required: false }));
    }

    #[test]
    fn residual_tail_under_250ms_waits_when_not_final() {
        assert_eq!(next_step(5_000, 5_200, false, &cfg()), Step::Wait);
    }

    #[test]
    fn residual_tail_fully_consumed_completes_on_final_drain() {
        // Every buffered sample is already past a finished segment boundary.
        let step = next_step(5_000, 5_000, true, &cfg());
        assert_eq!(step, Step::Stop(StopOutcome { complete: true, fallback_required: false }));
    }

    #[test]
    fn residual_tail_under_250ms_still_forces_a_fallback_on_final_drain() {
        // A trailing residual too short to safely cut its own segment is
        // lost incremental work, not silently dropped: it must force a
        // fallback instead of being reported complete.
        let step = next_step(5_000, 5_200, true, &cfg());
        assert_eq!(step, Step::Stop(StopOutcome { complete: false, fallback_required: true }));
    }

    #[test]
    fn residual_tail_over_250ms_still_produces_a_final_segment() {
        let step = next_step(5_000, 5_400, true, &cfg());
        assert_eq!(step, Step::Produce { start_ms: 5_000, desired_end_ms: 5_400 });
    }

    // --- next_step: waiting for a full target buffer --------------------

    #[test]
    fn waits_for_a_full_target_buffer_before_producing() {
        assert_eq!(next_step(0, 4_000, false, &cfg()), Step::Wait);
        assert_eq!(
            next_step(0, 5_000, false, &cfg()),
            Step::Produce { start_ms: 0, desired_end_ms: 5_000 }
        );
    }

    // --- next_step: live vs final backlog overflow (unified policy) -----

    #[test]
    fn live_backlog_overflow_stops_immediately_with_fallback() {
        // backlog_limit_ms = max_ms(10_000) * max_queue(2) = 20_000
        let step = next_step(0, 20_001, false, &cfg());
        assert_eq!(
            step,
            Step::Stop(StopOutcome { complete: false, fallback_required: true }),
            "live backlog overflow must stop scheduling immediately, not just flag fallback"
        );
    }

    #[test]
    fn live_backlog_at_the_limit_still_schedules() {
        let step = next_step(0, 20_000, false, &cfg());
        assert!(matches!(step, Step::Produce { .. }));
    }

    #[test]
    fn final_backlog_overflow_stops_with_fallback() {
        let step = next_step(0, 10_001, true, &cfg());
        assert_eq!(
            step,
            Step::Stop(StopOutcome { complete: false, fallback_required: true })
        );
    }

    #[test]
    fn final_drain_targets_max_ms_not_target_ms() {
        let step = next_step(0, 9_000, true, &cfg());
        assert_eq!(step, Step::Produce { start_ms: 0, desired_end_ms: 9_000 });
    }

    // --- after_boundary ---------------------------------------------------

    #[test]
    fn boundary_that_advances_proceeds() {
        assert_eq!(after_boundary(0, 5_000, 5_000, false), BoundaryStep::Proceed);
    }

    #[test]
    fn stuck_boundary_waits_when_not_final() {
        assert_eq!(after_boundary(5_000, 5_000, 5_000, false), BoundaryStep::Wait);
    }

    #[test]
    fn stuck_boundary_stops_on_final_drain() {
        assert_eq!(
            after_boundary(5_000, 5_000, 5_000, true),
            BoundaryStep::Stop(StopOutcome { complete: true, fallback_required: false })
        );
    }

    // --- classify_slice: empty/failed slice -------------------------------

    fn slice(sample_count: usize) -> WavSlice {
        WavSlice { start_ms: 0, end_ms: 5_000, sample_count, peak: 0.0, rms: 0.0 }
    }

    #[test]
    fn empty_slice_waits_when_not_final() {
        assert!(matches!(
            classify_slice(Ok(slice(0)), false, 5_000, 0),
            SliceStep::Wait
        ));
    }

    #[test]
    fn empty_slice_stops_on_final_drain() {
        let step = classify_slice(Ok(slice(0)), true, 5_000, 5_000);
        assert!(matches!(
            step,
            SliceStep::Stop(StopOutcome { complete: true, fallback_required: false })
        ));
    }

    #[test]
    fn failed_slice_waits_when_not_final() {
        assert!(matches!(
            classify_slice(Err(anyhow::anyhow!("boom")), false, 5_000, 0),
            SliceStep::Wait
        ));
    }

    #[test]
    fn failed_slice_fails_the_segment_on_final_drain() {
        let step = classify_slice(Err(anyhow::anyhow!("boom")), true, 5_000, 0);
        assert!(matches!(step, SliceStep::Failed(msg) if msg.contains("boom")));
    }

    #[test]
    fn nonempty_slice_is_used() {
        assert!(matches!(classify_slice(Ok(slice(10)), false, 5_000, 0), SliceStep::Use(_)));
    }

    // --- run(): full driver, exercised against real WAV fixtures --------

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("pickscribe-incremental-{name}-{id}"))
    }

    /// A tone loud enough that low-energy boundary selection lands near the
    /// desired cut point instead of drifting to the fixture's silent tail.
    fn tone_samples(count: usize) -> Vec<i16> {
        (0..count)
            .map(|i| ((i % 200) as i32 - 100) as i16 * 300)
            .collect()
    }

    fn write_fixture(path: &Path, duration_ms: u64) {
        let sample_count = (audio_segments::ms_to_sample(duration_ms)) as usize;
        audio_segments::write_wav(path, &tone_samples(sample_count)).unwrap();
    }

    fn append_fixture(path: &Path, extra_ms: u64) {
        let existing = audio_segments::read_samples(path, 0, u64::MAX / 2).unwrap();
        let mut samples = existing;
        samples.extend(tone_samples(audio_segments::ms_to_sample(extra_ms) as usize));
        audio_segments::write_wav(path, &samples).unwrap();
    }

    #[derive(Default)]
    struct FakeHostState {
        control: Control,
        transcripts: Deque<Result<String>>,
        cleanup_enabled: bool,
        keep_audio: bool,
        published: Vec<RecordingSession>,
        transcribed_jobs: Vec<SegmentJob>,
        segment_files_present_after_transcribe: Vec<bool>,
        cleanup_artifacts_called: bool,
        cancel_after_n_transcriptions: Option<usize>,
        stop_after_n_transcriptions: Option<usize>,
    }

    struct FakeHost {
        state: Mutex<FakeHostState>,
        temp_dir: PathBuf,
    }

    impl FakeHost {
        fn new(temp_dir: PathBuf) -> Self {
            fs::create_dir_all(&temp_dir).unwrap();
            Self {
                state: Mutex::new(FakeHostState::default()),
                temp_dir,
            }
        }

        fn with_transcripts(self, transcripts: Vec<Result<String>>) -> Self {
            self.state.lock().unwrap().transcripts = transcripts.into();
            self
        }

        fn with_control(self, control: Control) -> Self {
            self.state.lock().unwrap().control = control;
            self
        }

        fn with_cleanup_enabled(self) -> Self {
            self.state.lock().unwrap().cleanup_enabled = true;
            self
        }

        fn with_keep_audio(self) -> Self {
            self.state.lock().unwrap().keep_audio = true;
            self
        }

        fn cancel_after_n_transcriptions(self, n: usize) -> Self {
            self.state.lock().unwrap().cancel_after_n_transcriptions = Some(n);
            self
        }

        fn stop_after_n_transcriptions(self, n: usize) -> Self {
            self.state.lock().unwrap().stop_after_n_transcriptions = Some(n);
            self
        }
    }

    impl IncrementalHost for FakeHost {
        fn control(&self) -> Control {
            self.state.lock().unwrap().control
        }

        fn keep_audio(&self) -> bool {
            self.state.lock().unwrap().keep_audio
        }

        fn segment_path(&self, segment_id: u64) -> PathBuf {
            self.temp_dir.join(format!("segment-{segment_id:04}.wav"))
        }

        fn transcribe(&mut self, job: &SegmentJob) -> Result<String> {
            let mut state = self.state.lock().unwrap();
            state.transcribed_jobs.push(job.clone());
            state
                .segment_files_present_after_transcribe
                .push(job.audio_path.exists());
            let result = state
                .transcripts
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()));
            if let Some(threshold) = state.cancel_after_n_transcriptions
                && state.transcribed_jobs.len() >= threshold
            {
                state.control = Control::Cancelled;
            }
            if let Some(threshold) = state.stop_after_n_transcriptions
                && state.transcribed_jobs.len() >= threshold
                && state.control == Control::Continue
            {
                state.control = Control::Stopping;
            }
            result
        }

        fn try_queue_cleanup(&mut self, _segment: TranscriptSegment) -> bool {
            self.state.lock().unwrap().cleanup_enabled
        }

        fn drain_cleanup(&mut self) -> Vec<TranscriptSegment> {
            Vec::new()
        }

        fn publish(&mut self, session: &RecordingSession) {
            self.state.lock().unwrap().published.push(session.clone());
        }

        fn cleanup_artifacts(&mut self) {
            self.state.lock().unwrap().cleanup_artifacts_called = true;
        }
    }

    #[test]
    fn run_completes_after_the_final_drain_consumes_all_buffered_audio() {
        let dir = temp_dir("complete");
        let audio = dir.join("recording.wav");
        // 800ms beyond one target segment guarantees a >250ms residual
        // regardless of where the low-energy boundary snaps, so the final
        // drain always has a deterministic tail segment to produce.
        write_fixture(&audio, 5_800);
        // Stopping flips on right after the first segment's STT returns,
        // simulating a real stop landing while the worker is mid-flight.
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("hello world".into()), Ok("goodbye".into())])
            .with_control(Control::Continue)
            .stop_after_n_transcriptions(1);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(outcome.complete);
                assert!(!outcome.fallback_required);
                assert!(outcome.session.final_raw_text().starts_with("hello world"));
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }

    #[test]
    fn run_completes_with_no_segments_when_stopped_before_any_audio() {
        let dir = temp_dir("stop-before-first-segment");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 0);
        let mut host = FakeHost::new(dir.join("segments")).with_control(Control::Stopping);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(outcome.complete);
                assert!(!outcome.fallback_required);
                assert!(outcome.session.segments.is_empty());
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }

    #[test]
    fn run_forces_a_fallback_when_stopped_with_only_a_sub_250ms_residual() {
        let dir = temp_dir("stop-with-short-residual");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 100);
        let mut host = FakeHost::new(dir.join("segments")).with_control(Control::Stopping);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(!outcome.complete);
                assert!(outcome.fallback_required);
                assert!(outcome.session.segments.is_empty());
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }

    #[test]
    fn run_falls_back_when_a_segment_transcription_fails() {
        let dir = temp_dir("segment-failure");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Err(anyhow::anyhow!("stt crashed"))])
            .with_control(Control::Continue);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(!outcome.complete);
                assert!(outcome.fallback_required);
                assert_eq!(outcome.session.segments.len(), 1);
                assert_eq!(
                    outcome.session.segments[0].status,
                    TranscriptSegmentStatus::Failed
                );
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }

    #[test]
    fn run_stops_immediately_on_live_backlog_overflow_without_transcribing_more() {
        let dir = temp_dir("live-backlog-overflow");
        let audio = dir.join("recording.wav");
        // backlog_limit_ms for cfg() = max_ms(10_000) * max_queue(2) = 20_000
        write_fixture(&audio, 20_500);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("only one".into())])
            .with_control(Control::Continue);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(!outcome.complete);
                assert!(outcome.fallback_required);
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
        let jobs = host.state.lock().unwrap().transcribed_jobs.len();
        assert_eq!(
            jobs, 0,
            "live backlog overflow must stop before transcribing another segment"
        );
    }

    #[test]
    fn run_cancels_mid_stt_without_processing_the_result() {
        let dir = temp_dir("cancel-mid-stt");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("should be discarded".into())])
            .with_control(Control::Continue)
            .cancel_after_n_transcriptions(1);

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(outcome) => {
                assert!(!outcome.complete);
                assert!(!outcome.fallback_required);
                assert!(
                    outcome
                        .session
                        .segments
                        .iter()
                        .all(|s| s.status != TranscriptSegmentStatus::RawReady),
                    "cancelled segment result must not be applied"
                );
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }

    #[test]
    fn run_returns_abandoned_and_cleans_up_without_publishing() {
        let dir = temp_dir("abandoned");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments")).with_control(Control::Abandoned);

        let result = run(&mut host, &audio, "session-1", cfg());

        assert!(matches!(result, RunResult::Abandoned));
        assert!(host.state.lock().unwrap().cleanup_artifacts_called);
    }

    #[test]
    fn run_deletes_segment_files_after_transcribe_unless_keep_audio() {
        let dir = temp_dir("keep-audio");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("kept".into()), Ok("tail".into())])
            .with_control(Control::Continue)
            .stop_after_n_transcriptions(1)
            .with_keep_audio();

        let _ = run(&mut host, &audio, "session-1", cfg());

        let state = host.state.lock().unwrap();
        assert!(state.segment_files_present_after_transcribe[0]);
        let segment_path = host.segment_path(1);
        assert!(segment_path.exists(), "keep_audio must retain segment WAVs");
    }

    #[test]
    fn run_removes_segment_files_after_transcribe_by_default() {
        let dir = temp_dir("no-keep-audio");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("gone".into()), Ok("tail".into())])
            .with_control(Control::Continue)
            .stop_after_n_transcriptions(1);

        let _ = run(&mut host, &audio, "session-1", cfg());

        assert!(!host.segment_path(1).exists());
    }

    #[test]
    fn run_marks_finished_segments_cleaning_only_when_cleanup_accepts_them() {
        let dir = temp_dir("cleanup-queue");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("clean me".into()), Ok("tail".into())])
            .with_control(Control::Continue)
            .stop_after_n_transcriptions(1)
            .with_cleanup_enabled();

        let result = run(&mut host, &audio, "session-1", cfg());

        let RunResult::Finished(outcome) = result else {
            panic!("expected Finished");
        };
        // The final published state always reflects the latest known
        // status; cleanup completion itself is a host (adapter) concern.
        let published = host.state.lock().unwrap().published.clone();
        assert!(
            published
                .iter()
                .flat_map(|s| s.segments.iter())
                .any(|s| s.status == TranscriptSegmentStatus::Cleaning),
            "queued segment must be published as Cleaning before results drain"
        );
        assert!(!outcome.session.segments.is_empty());
    }

    #[test]
    fn run_does_not_queue_cleanup_for_empty_transcripts() {
        let dir = temp_dir("empty-transcript-no-cleanup");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 5_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("   ".into()), Ok("   ".into())])
            .with_control(Control::Continue)
            .stop_after_n_transcriptions(1)
            .with_cleanup_enabled();

        let _ = run(&mut host, &audio, "session-1", cfg());

        let published = host.state.lock().unwrap().published.clone();
        assert!(
            published
                .iter()
                .flat_map(|s| s.segments.iter())
                .all(|s| s.status != TranscriptSegmentStatus::Cleaning),
            "blank/low-energy transcripts must not be queued for cleanup"
        );
    }

    #[test]
    fn run_waits_for_more_audio_then_produces_once_the_target_buffer_fills() {
        let dir = temp_dir("growing-audio");
        let audio = dir.join("recording.wav");
        write_fixture(&audio, 4_000);
        let mut host = FakeHost::new(dir.join("segments"))
            .with_transcripts(vec![Ok("first".into())])
            .with_control(Control::Continue)
            .cancel_after_n_transcriptions(1);

        std::thread::spawn({
            let audio = audio.clone();
            move || {
                std::thread::sleep(Duration::from_millis(100));
                append_fixture(&audio, 2_000);
            }
        });

        let result = run(&mut host, &audio, "session-1", cfg());

        match result {
            RunResult::Finished(_) => {
                assert_eq!(host.state.lock().unwrap().transcribed_jobs.len(), 1);
            }
            RunResult::Abandoned => panic!("expected Finished"),
        }
    }
}
