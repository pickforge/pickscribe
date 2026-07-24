use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use super::find_command;

const SAMPLE_RATE: u32 = 48_000;
const VOLUME: f32 = 0.32;

pub enum Cue {
    Start,
    Stop,
    Error,
}

fn sounds_dir() -> PathBuf {
    crate::config::data_dir().join("sounds")
}

/// Ensure the synthesized cue files exist, returning the path for `cue`.
fn cue_path(cue: &Cue) -> Result<PathBuf> {
    let dir = sounds_dir();
    fs::create_dir_all(&dir).context("creating sounds dir")?;
    let (name, spec): (&str, Vec<(f32, f32, f32)>) = match cue {
        // (start_hz, end_hz, seconds) segments, played back to back.
        Cue::Start => ("start.wav", vec![(523.25, 659.25, 0.07), (659.25, 880.0, 0.09)]),
        Cue::Stop => ("stop.wav", vec![(880.0, 659.25, 0.07), (659.25, 440.0, 0.10)]),
        Cue::Error => ("error.wav", vec![(330.0, 262.0, 0.16)]),
    };
    let path = dir.join(name);
    if !path.is_file() {
        write_wav(&path, &spec)?;
    }
    Ok(path)
}

fn write_wav(path: &PathBuf, segments: &[(f32, f32, f32)]) -> Result<()> {
    let mut samples: Vec<i16> = Vec::new();
    for &(from_hz, to_hz, secs) in segments {
        let count = (SAMPLE_RATE as f32 * secs) as usize;
        let mut phase = 0f32;
        for i in 0..count {
            let t = i as f32 / count as f32;
            let hz = from_hz + (to_hz - from_hz) * t;
            phase += std::f32::consts::TAU * hz / SAMPLE_RATE as f32;
            // Short attack/release envelope to avoid clicks.
            let env = (t * 24.0).min(1.0) * ((1.0 - t) * 6.0).min(1.0);
            let value = phase.sin() * env * VOLUME;
            samples.push((value * i16::MAX as f32) as i16);
        }
    }

    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).context("writing cue wav")?;
    Ok(())
}

/// Play a cue without blocking the pipeline. Failures are silent — sound is
/// feedback, never a dependency.
pub fn play(cue: Cue) {
    std::thread::spawn(move || {
        let Ok(path) = cue_path(&cue) else { return };
        for player in ["pw-play", "paplay", "aplay", "afplay"] {
            if let Some(program) = find_command(player) {
                let _ = Command::new(program)
                    .arg(&path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                return;
            }
        }
    });
}
