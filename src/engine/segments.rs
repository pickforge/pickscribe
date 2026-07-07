use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptSegmentStatus {
    Recording,
    Transcribing,
    RawReady,
    Cleaning,
    Cleaned,
    Provisional,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: TranscriptSegmentStatus,
    pub raw_text: String,
    pub cleaned_text: Option<String>,
    pub error: Option<String>,
}

impl TranscriptSegment {
    pub fn raw_ready(id: u64, start_ms: u64, end_ms: u64, raw_text: impl Into<String>) -> Self {
        Self {
            id,
            start_ms,
            end_ms,
            status: TranscriptSegmentStatus::RawReady,
            raw_text: raw_text.into(),
            cleaned_text: None,
            error: None,
        }
    }

    pub fn failed(
        id: u64,
        start_ms: u64,
        end_ms: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id,
            start_ms,
            end_ms,
            status: TranscriptSegmentStatus::Failed,
            raw_text: String::new(),
            cleaned_text: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSession {
    pub id: String,
    pub segments: Vec<TranscriptSegment>,
}

impl RecordingSession {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            segments: Vec::new(),
        }
    }

    pub fn upsert_segment(&mut self, segment: TranscriptSegment) {
        if let Some(existing) = self.segments.iter_mut().find(|item| item.id == segment.id) {
            *existing = segment;
        } else {
            self.segments.push(segment);
        }
        self.segments
            .sort_by_key(|segment| (segment.start_ms, segment.end_ms, segment.id));
    }

    pub fn final_raw_text(&self) -> String {
        assemble_raw_text(&self.segments)
    }

    pub fn final_cleaned_text(&self) -> Option<String> {
        assemble_cleaned_text(&self.segments)
    }
}

pub fn assemble_raw_text(segments: &[TranscriptSegment]) -> String {
    let ordered = ordered_segments(segments);
    merge_texts(ordered.iter().map(|segment| segment.raw_text.as_str()))
}

pub fn assemble_cleaned_text(segments: &[TranscriptSegment]) -> Option<String> {
    let ordered = ordered_segments(segments);
    let has_cleaned = ordered
        .iter()
        .any(|segment| segment.cleaned_text.as_deref().is_some_and(has_text));
    if !has_cleaned {
        return None;
    }

    let texts = ordered.iter().filter_map(|segment| {
        segment
            .cleaned_text
            .as_deref()
            .filter(|text| has_text(text))
            .or_else(|| has_text(&segment.raw_text).then_some(segment.raw_text.as_str()))
    });
    Some(merge_texts(texts))
}

pub fn merge_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    let mut merged: Vec<String> = Vec::new();
    for text in texts {
        let incoming = split_words(text);
        if incoming.is_empty() {
            continue;
        }
        if merged.is_empty() {
            merged.extend(incoming);
            continue;
        }

        let overlap = overlap_word_count(&merged, &incoming);
        if overlap > 0 {
            merged.extend(incoming.into_iter().skip(overlap));
            continue;
        }

        if should_replace_partial_tail(&merged, &incoming) {
            merged.pop();
        }
        merged.extend(incoming);
    }
    merged.join(" ").trim().to_string()
}

fn ordered_segments(segments: &[TranscriptSegment]) -> Vec<&TranscriptSegment> {
    let mut ordered: Vec<&TranscriptSegment> = segments.iter().collect();
    ordered.sort_by_key(|segment| (segment.start_ms, segment.end_ms, segment.id));
    ordered
}

fn has_text(text: &str) -> bool {
    !text.trim().is_empty()
}

fn split_words(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if is_non_speech_marker(trimmed) {
        return Vec::new();
    }
    trimmed.split_whitespace().map(ToOwned::to_owned).collect()
}

fn overlap_word_count(left: &[String], right: &[String]) -> usize {
    let max = left.len().min(right.len());
    for size in (1..=max).rev() {
        if words_match(&left[left.len() - size..], &right[..size]) {
            return size;
        }
    }
    0
}

fn words_match(left: &[String], right: &[String]) -> bool {
    left.iter().zip(right).all(|(left, right)| {
        let left = normalize_word(left);
        let right = normalize_word(right);
        !left.is_empty() && left == right
    })
}

fn should_replace_partial_tail(left: &[String], right: &[String]) -> bool {
    let Some(left_original) = left.last() else {
        return false;
    };
    if !left_original.trim_end().ends_with('-') {
        return false;
    }
    let left = normalize_word(left_original);
    let Some(right) = right.first().map(|word| normalize_word(word)) else {
        return false;
    };
    left.len() >= 5 && right.len() > left.len() && right.starts_with(&left)
}

fn normalize_word(word: &str) -> String {
    word.trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_lowercase()
}

fn is_non_speech_marker(text: &str) -> bool {
    matches!(
        text.to_ascii_uppercase().as_str(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: u64, start_ms: u64, raw_text: &str) -> TranscriptSegment {
        TranscriptSegment::raw_ready(id, start_ms, start_ms + 5_000, raw_text)
    }

    #[test]
    fn status_serializes_as_frontend_payload_strings() {
        assert_eq!(
            serde_json::to_string(&TranscriptSegmentStatus::Recording).unwrap(),
            "\"recording\""
        );
        assert_eq!(
            serde_json::to_string(&TranscriptSegmentStatus::RawReady).unwrap(),
            "\"rawReady\""
        );
    }

    #[test]
    fn session_upserts_and_orders_segments() {
        let mut session = RecordingSession::new("session-1");
        session.upsert_segment(segment(2, 5_000, "second"));
        session.upsert_segment(segment(1, 0, "first"));
        session.upsert_segment(segment(2, 5_000, "replacement"));

        assert_eq!(
            session
                .segments
                .iter()
                .map(|segment| (segment.id, segment.raw_text.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "first"), (2, "replacement")]
        );
    }

    #[test]
    fn raw_assembly_dedupes_normalized_overlap() {
        let segments = vec![
            segment(1, 0, "Hello, world. This is"),
            segment(2, 4_000, "world this is a test"),
            segment(3, 8_000, "a test for PickScribe"),
        ];

        assert_eq!(
            assemble_raw_text(&segments),
            "Hello, world. This is a test for PickScribe"
        );
    }

    #[test]
    fn raw_assembly_preserves_repeated_words_while_deduping_overlap() {
        assert_eq!(merge_texts(["go go now", "go now please"]), "go go now please");
    }

    #[test]
    fn raw_assembly_replaces_partial_tail_word() {
        assert_eq!(
            merge_texts(["this is an increm-", "incremental transcript"]),
            "this is an incremental transcript"
        );
    }

    #[test]
    fn raw_assembly_preserves_complete_prefix_words() {
        assert_eq!(
            merge_texts(["we were there", "therefore we left"]),
            "we were there therefore we left"
        );
    }

    #[test]
    fn raw_assembly_skips_empty_and_silent_chunks() {
        assert_eq!(
            merge_texts(["", "   ", "hello there", "[SILENCE]", "there friend"]),
            "hello there friend"
        );
    }

    #[test]
    fn cleaned_assembly_uses_cleaned_segments_when_available() {
        let mut first = segment(1, 0, "raw one");
        first.cleaned_text = Some("Clean one".into());
        let second = segment(2, 5_000, "one two");

        assert_eq!(
            assemble_cleaned_text(&[second, first]),
            Some("Clean one two".into())
        );
    }

    #[test]
    fn cleaned_assembly_is_none_without_cleaned_text() {
        assert_eq!(assemble_cleaned_text(&[segment(1, 0, "raw")]), None);
    }
}
