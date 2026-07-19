//! Deep module for the recording pipeline's lifecycle decisions.
//!
//! `engine.rs` owns every effect (recorder, incremental/STT, cleanup,
//! delivery, history, Tauri events) and every entry adapter (command, tray,
//! single-instance) routes through it. What was hard to characterize
//! without Tauri was the *policy* buried inside that effectful code:
//! whether a toggle should start/stop/no-op, whether a pipeline stage's
//! result still belongs to the session the UI is showing, and what a
//! terminal effect result should mean for the visible state.
//!
//! This module extracts exactly that policy as plain functions over plain
//! data, so it can be characterized without an `AppHandle`. It does not
//! touch incremental scheduling (CAND-1's scope) and does not change the
//! `Stage` set or emission order — `engine.rs` still decides *when* to call
//! these functions and still performs every effect itself.

use pickscribe::engine::cleanup::CleanupOutcome;
use pickscribe::history::HistoryEntry;

use crate::engine::Stage;

/// What a toggle request should do given the pipeline's current stage.
///
/// Every entry adapter (command, tray, single-instance `--toggle`) reads
/// the current stage and calls this before touching any effect. Toggling
/// while the pipeline is busy (`Transcribing`/`Cleaning`/`Pasting`) is
/// ignored rather than queued or restarted — this is what keeps a second,
/// overlapping toggle from a different adapter (or a fast double-press)
/// from starting a duplicate recording on top of one already in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    Start,
    Stop,
    Ignore,
}

pub fn toggle_action(stage: Stage) -> ToggleAction {
    match stage {
        Stage::Idle => ToggleAction::Start,
        Stage::Recording => ToggleAction::Stop,
        Stage::Transcribing | Stage::Cleaning | Stage::Pasting => ToggleAction::Ignore,
    }
}

/// The engine's live session record, as far as a pipeline checkpoint needs
/// to see it: which session id is current, and whether it has already been
/// cancelled.
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub id: String,
    pub cancelled: bool,
}

/// Whether a pipeline stage tagged with `session_id` (and its own
/// `token_cancelled` flag) may still apply its result to visible state.
///
/// Every checkpoint in the stop pipeline — after the recorder stops, after
/// transcript resolution, after cleanup, after delivery, and at terminal
/// projection — asks this same question before touching `StatePayload`.
/// Centralizing it here means:
///
/// - a stop that races session registration (`active` is `None`) is never
///   current, so it silently no-ops instead of touching whatever session
///   happens to be live next (characterizes "stop before init finishes");
/// - a cancellation flips `cancelled` (on the snapshot or the stage's own
///   token) at any point, and every later checkpoint stops applying results
///   from that point on (characterizes "cancel at every pipeline stage");
/// - a new session starting after a cancel/completion changes `active.id`,
///   so a late result from the old session_id is rejected even if nothing
///   was ever explicitly cancelled (characterizes "stale completion after
///   cancel/new session").
pub fn session_is_current(
    active: Option<&SessionSnapshot>,
    session_id: &str,
    token_cancelled: bool,
) -> bool {
    if token_cancelled {
        return false;
    }
    active.is_some_and(|session| session.id == session_id && !session.cancelled)
}

/// Fixed message paired with the "no speech detected" terminal state, kept
/// here so the pipeline and its tests share one source of truth.
pub const NO_SPEECH_MESSAGE: &str = "No speech detected";

/// Terminal projection for a recorder or transcription failure: the error
/// surfaces, no completion message is shown, and (matching current
/// behavior) any earlier `last_entry` is left untouched rather than
/// cleared.
pub fn failure_outcome(err: impl Into<String>) -> (Option<String>, Option<String>) {
    (Some(err.into()), None)
}

/// Terminal projection when the transcript came back empty: no error, a
/// fixed advisory message, `last_entry` left untouched.
pub fn no_speech_outcome() -> (Option<String>, Option<String>) {
    (None, Some(NO_SPEECH_MESSAGE.to_string()))
}

/// Terminal projection once a transcript has been cleaned (or not) and
/// delivery has been attempted.
#[derive(Debug, Clone)]
pub struct DeliveryOutcome {
    pub error: Option<String>,
    pub message: String,
    pub last_entry: Option<HistoryEntry>,
}

/// Combine delivery and cleanup results into the terminal
/// error/message/history-entry triple.
///
/// `history_entry` is the effect's own result: `Some` on a successful
/// insert, `None` on a lookup/insert failure. History failure never
/// changes `error` or `message` — it only ever changes whether `last_entry`
/// is populated — which is the invariant "history failure" characterizes.
///
/// `paste_error` is surfaced independently of `cleanup`: a cleanup failure
/// with a successful raw-text delivery reports no error, only an advisory
/// message ("cleanup failure with raw delivery"); a paste failure reports
/// an error regardless of whether cleanup succeeded.
pub fn delivery_outcome(
    paste_error: Option<String>,
    cleanup: &CleanupOutcome,
    history_entry: Option<HistoryEntry>,
) -> DeliveryOutcome {
    let message = if cleanup.cleaned {
        "Cleaned and pasted"
    } else if cleanup.error.is_some() {
        "Pasted raw transcript; cleanup unavailable"
    } else {
        "Pasted raw transcript"
    };

    DeliveryOutcome {
        error: paste_error,
        message: message.into(),
        last_entry: history_entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleanup_outcome(cleaned: bool, error: Option<&str>) -> CleanupOutcome {
        CleanupOutcome {
            text: "transcript".into(),
            provider: "ollama".into(),
            model: "qwen2.5:14b".into(),
            cleaned,
            error: error.map(str::to_string),
        }
    }

    fn history_entry(id: i64) -> HistoryEntry {
        HistoryEntry {
            id,
            created_at: 0,
            duration_ms: 1_000,
            raw_text: "transcript".into(),
            cleaned_text: None,
            provider: "ollama".into(),
            model: "qwen2.5:14b".into(),
            language: "en".into(),
            source_file: None,
            segments_json: None,
            word_count: 1,
        }
    }

    // --- duplicate start/toggle -------------------------------------------

    #[test]
    fn toggle_starts_only_from_idle() {
        assert_eq!(toggle_action(Stage::Idle), ToggleAction::Start);
    }

    #[test]
    fn toggle_stops_only_from_recording_never_starting_a_duplicate() {
        assert_eq!(toggle_action(Stage::Recording), ToggleAction::Stop);
    }

    #[test]
    fn toggle_is_ignored_while_the_pipeline_is_busy() {
        for stage in [Stage::Transcribing, Stage::Cleaning, Stage::Pasting] {
            assert_eq!(
                toggle_action(stage),
                ToggleAction::Ignore,
                "stage: {stage:?}"
            );
        }
    }

    // --- stop before init finishes / session-current at every checkpoint --

    #[test]
    fn no_active_session_is_never_current() {
        // Characterizes "stop before init finishes": a checkpoint that
        // fires before the session finished registering sees no active
        // session at all and must no-op rather than adopt whichever
        // session comes next.
        assert!(!session_is_current(None, "session-1", false));
    }

    #[test]
    fn matching_uncancelled_session_is_current() {
        let active = SessionSnapshot {
            id: "session-1".into(),
            cancelled: false,
        };
        assert!(session_is_current(Some(&active), "session-1", false));
    }

    #[test]
    fn cancelled_session_record_is_never_current() {
        let active = SessionSnapshot {
            id: "session-1".into(),
            cancelled: true,
        };
        assert!(!session_is_current(Some(&active), "session-1", false));
    }

    #[test]
    fn cancelled_stage_token_is_never_current_even_if_session_record_is_fresh() {
        let active = SessionSnapshot {
            id: "session-1".into(),
            cancelled: false,
        };
        assert!(!session_is_current(Some(&active), "session-1", true));
    }

    #[test]
    fn stale_completion_from_a_superseded_session_is_never_current() {
        // Characterizes "stale completion after cancel/new session": a new
        // session is live, but a checkpoint tagged with the old session id
        // (e.g. a slow effect finishing after the user cancelled and
        // started again) must not apply.
        let active = SessionSnapshot {
            id: "session-2".into(),
            cancelled: false,
        };
        assert!(!session_is_current(Some(&active), "session-1", false));
    }

    #[test]
    fn every_pipeline_checkpoint_shares_the_same_current_session_decision() {
        // "Cancel at every pipeline stage" is one decision point (this
        // function) reused everywhere; this enumerates the checkpoints by
        // name to document that a cancellation reaching any of them stops
        // that stage's result from applying.
        let checkpoints = [
            "after recorder stop",
            "after transcript resolution",
            "after cleanup",
            "after delivery",
            "at terminal projection",
        ];
        let cancelled = SessionSnapshot {
            id: "session-1".into(),
            cancelled: true,
        };
        for checkpoint in checkpoints {
            assert!(
                !session_is_current(Some(&cancelled), "session-1", false),
                "checkpoint: {checkpoint}"
            );
        }
    }

    // --- terminal outcome projection ---------------------------------------

    #[test]
    fn recorder_or_transcription_failure_surfaces_error_without_a_message() {
        let (error, message) = failure_outcome("device busy");
        assert_eq!(error.as_deref(), Some("device busy"));
        assert_eq!(message, None);
    }

    #[test]
    fn empty_transcript_surfaces_advisory_message_without_an_error() {
        let (error, message) = no_speech_outcome();
        assert_eq!(error, None);
        assert_eq!(message.as_deref(), Some(NO_SPEECH_MESSAGE));
    }

    #[test]
    fn successful_cleanup_reports_cleaned_message() {
        let outcome = delivery_outcome(None, &cleanup_outcome(true, None), Some(history_entry(1)));
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.message, "Cleaned and pasted");
        assert_eq!(outcome.last_entry.map(|e| e.id), Some(1));
    }

    #[test]
    fn cleanup_failure_with_raw_delivery_reports_no_error() {
        let outcome = delivery_outcome(
            None,
            &cleanup_outcome(false, Some("model unavailable")),
            Some(history_entry(1)),
        );
        assert_eq!(outcome.error, None);
        assert_eq!(
            outcome.message,
            "Pasted raw transcript; cleanup unavailable"
        );
    }

    #[test]
    fn cleanup_failure_and_delivery_failure_both_surface() {
        let outcome = delivery_outcome(
            Some("paste failed".into()),
            &cleanup_outcome(false, Some("model unavailable")),
            Some(history_entry(1)),
        );
        assert_eq!(outcome.error.as_deref(), Some("paste failed"));
        assert_eq!(
            outcome.message,
            "Pasted raw transcript; cleanup unavailable"
        );
    }

    #[test]
    fn delivery_failure_with_successful_cleanup_still_surfaces_error() {
        let outcome = delivery_outcome(
            Some("paste failed".into()),
            &cleanup_outcome(true, None),
            Some(history_entry(1)),
        );
        assert_eq!(outcome.error.as_deref(), Some("paste failed"));
        assert_eq!(outcome.message, "Cleaned and pasted");
    }

    #[test]
    fn history_failure_clears_last_entry_without_affecting_error_or_message() {
        let with_history =
            delivery_outcome(None, &cleanup_outcome(true, None), Some(history_entry(1)));
        let without_history = delivery_outcome(None, &cleanup_outcome(true, None), None);

        assert_eq!(without_history.error, with_history.error);
        assert_eq!(without_history.message, with_history.message);
        assert!(without_history.last_entry.is_none());
    }
}
