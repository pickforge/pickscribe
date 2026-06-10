use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Incrementally reads a growing 16-bit mono WAV file and yields a peak level
/// (0.0–1.0) for the newest audio since the last poll. Drives the live waveform.
pub struct LevelMeter {
    file: File,
    cursor: u64,
}

const WAV_HEADER_BYTES: u64 = 44;

impl LevelMeter {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            file,
            cursor: WAV_HEADER_BYTES,
        })
    }

    /// Peak amplitude of samples written since the last call. Returns None
    /// when no new audio is available yet.
    pub fn poll(&mut self) -> Option<f32> {
        let len = self.file.metadata().ok()?.len();
        if len <= self.cursor + 2 {
            return None;
        }
        // Cap each poll at ~100ms of 16kHz mono s16 audio (3200 bytes).
        let available = len - self.cursor;
        let to_read = available.min(3200) as usize & !1;
        // Always read the newest window, skipping anything older.
        let start = len - to_read as u64;
        if self.file.seek(SeekFrom::Start(start)).is_err() {
            return None;
        }
        let mut buf = vec![0u8; to_read];
        self.file.read_exact(&mut buf).ok()?;
        self.cursor = len;

        let mut peak = 0i32;
        for chunk in buf.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
            peak = peak.max(sample.abs());
        }
        Some((peak as f32 / i16::MAX as f32).min(1.0))
    }
}
