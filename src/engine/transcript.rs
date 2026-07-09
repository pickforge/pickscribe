use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperSegment>,
}

#[derive(Deserialize)]
struct WhisperSegment {
    offsets: WhisperOffsets,
    text: String,
}

#[derive(Deserialize)]
struct WhisperOffsets {
    from: i64,
    to: i64,
}

pub fn parse_whisper_json(raw: &str) -> Result<Vec<FileSegment>> {
    let output: WhisperOutput = serde_json::from_str(raw).context("parsing whisper JSON")?;
    Ok(output
        .transcription
        .into_iter()
        .filter_map(|segment| {
            let text = segment.text.trim();
            if text.is_empty() || is_non_speech_marker(text) {
                return None;
            }
            Some(FileSegment {
                start_ms: segment.offsets.from,
                end_ms: segment.offsets.to,
                text: text.to_string(),
            })
        })
        .collect())
}

pub fn to_txt(segments: &[FileSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn to_srt(segments: &[FileSegment]) -> String {
    if segments.is_empty() {
        return String::new();
    }

    let cues = segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "{}\n{} --> {}\n{}",
                index + 1,
                format_srt_timestamp(segment.start_ms),
                format_srt_timestamp(segment.end_ms),
                segment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{cues}\n")
}

pub fn to_vtt(segments: &[FileSegment]) -> String {
    let cues = segments
        .iter()
        .map(|segment| {
            format!(
                "{} --> {}\n{}",
                format_vtt_timestamp(segment.start_ms),
                format_vtt_timestamp(segment.end_ms),
                segment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if cues.is_empty() {
        "WEBVTT\n\n".to_string()
    } else {
        format!("WEBVTT\n\n{cues}\n")
    }
}

fn is_non_speech_marker(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'))
}

fn format_srt_timestamp(ms: i64) -> String {
    format_timestamp(ms, ',')
}

fn format_vtt_timestamp(ms: i64) -> String {
    format_timestamp(ms, '.')
}

fn format_timestamp(ms: i64, separator: char) -> String {
    let ms = ms.max(0) as u64;
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let milliseconds = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{milliseconds:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_json_and_drops_non_speech_markers() -> Result<()> {
        let raw = r#"{
            "transcription": [
                {"offsets": {"from": 0, "to": 1420}, "text": "  Hello there.  "},
                {"offsets": {"from": 1420, "to": 2800}, "text": "[MUSIC]"},
                {"offsets": {"from": 2800, "to": 4310}, "text": "How are you?"}
            ]
        }"#;

        let segments = parse_whisper_json(raw)?;

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start_ms, 0);
        assert_eq!(segments[0].end_ms, 1_420);
        assert_eq!(segments[0].text, "Hello there.");
        assert_eq!(segments[1].text, "How are you?");
        assert_eq!(to_txt(&segments), "Hello there. How are you?");
        Ok(())
    }

    #[test]
    fn writes_srt_and_vtt_with_hour_rollover() {
        let segments = vec![
            FileSegment {
                start_ms: 0,
                end_ms: 1_234,
                text: "First cue".into(),
            },
            FileSegment {
                start_ms: 3_661_001,
                end_ms: 3_662_345,
                text: "Past an hour".into(),
            },
        ];

        assert_eq!(
            to_srt(&segments),
            "1\n00:00:00,000 --> 00:00:01,234\nFirst cue\n\n2\n01:01:01,001 --> 01:01:02,345\nPast an hour\n"
        );
        assert_eq!(
            to_vtt(&segments),
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.234\nFirst cue\n\n01:01:01.001 --> 01:01:02.345\nPast an hour\n"
        );
    }

    #[test]
    fn writes_empty_exports() {
        assert_eq!(to_txt(&[]), "");
        assert_eq!(to_srt(&[]), "");
        assert_eq!(to_vtt(&[]), "WEBVTT\n\n");
    }
}
