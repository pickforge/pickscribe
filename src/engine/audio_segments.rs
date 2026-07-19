use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

pub const WAV_HEADER_BYTES: u64 = 44;
pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
pub const BYTES_PER_SAMPLE: u64 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct WavSlice {
    pub start_ms: u64,
    pub end_ms: u64,
    pub sample_count: usize,
    pub peak: f32,
    pub rms: f32,
}

pub fn slice_wav(
    source: &Path,
    destination: &Path,
    start_ms: u64,
    end_ms: u64,
) -> Result<WavSlice> {
    if end_ms < start_ms {
        bail!("segment end {end_ms}ms is before start {start_ms}ms");
    }

    let (samples, actual_start_ms, actual_end_ms) = read_sample_window(source, start_ms, end_ms)?;
    write_wav(destination, &samples)?;
    let (peak, rms) = energy(&samples);
    Ok(WavSlice {
        start_ms: actual_start_ms,
        end_ms: actual_end_ms,
        sample_count: samples.len(),
        peak,
        rms,
    })
}

pub fn read_samples(source: &Path, start_ms: u64, end_ms: u64) -> Result<Vec<i16>> {
    read_sample_window(source, start_ms, end_ms).map(|(samples, _, _)| samples)
}

fn read_sample_window(source: &Path, start_ms: u64, end_ms: u64) -> Result<(Vec<i16>, u64, u64)> {
    let mut file = File::open(source).with_context(|| format!("opening {}", source.display()))?;
    let mut header = [0u8; WAV_HEADER_BYTES as usize];
    file.read_exact(&mut header)
        .with_context(|| format!("reading WAV header from {}", source.display()))?;
    let header_data_bytes = validate_header(&header)?;

    let file_data_bytes = file
        .metadata()
        .with_context(|| format!("stating {}", source.display()))?
        .len()
        .saturating_sub(WAV_HEADER_BYTES);
    let total_bytes = audio_data_bytes(&mut file, header_data_bytes, file_data_bytes)?;
    let available_samples = total_bytes / BYTES_PER_SAMPLE;
    let start_sample = ms_to_sample(start_ms).min(available_samples);
    let end_sample = ms_to_sample(end_ms).min(available_samples);
    let actual_start_ms = sample_to_ms(start_sample);
    let actual_end_ms = sample_to_ms(end_sample);
    if end_sample <= start_sample {
        return Ok((Vec::new(), actual_start_ms, actual_end_ms));
    }

    let data_len = (end_sample - start_sample) * BYTES_PER_SAMPLE;
    file.seek(SeekFrom::Start(
        WAV_HEADER_BYTES + start_sample * BYTES_PER_SAMPLE,
    ))?;
    let mut bytes = vec![0u8; data_len as usize];
    file.read_exact(&mut bytes)?;
    let samples = bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    Ok((samples, actual_start_ms, actual_end_ms))
}

pub fn write_wav(destination: &Path, samples: &[i16]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }

    let data_len = u32::try_from(
        samples
            .len()
            .checked_mul(BYTES_PER_SAMPLE as usize)
            .context("WAV segment is too large")?,
    )
    .context("WAV segment is too large")?;
    let mut file =
        File::create(destination).with_context(|| format!("creating {}", destination.display()))?;
    write_header(&mut file, data_len)?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

pub fn find_low_energy_boundary(samples: &[i16], target: usize, radius: usize) -> usize {
    if samples.is_empty() {
        return 0;
    }

    let start = target.saturating_sub(radius).min(samples.len() - 1);
    let end = target.saturating_add(radius).min(samples.len() - 1);
    let window = (SAMPLE_RATE_HZ as usize / 50).max(1);
    let mut best_index = target.min(samples.len() - 1);
    let mut best_energy = u64::MAX;

    for index in start..=end {
        let window_start = index.saturating_sub(window / 2);
        let window_end = (index + window / 2).min(samples.len() - 1);
        let mut energy = 0u64;
        for sample in &samples[window_start..=window_end] {
            energy += sample.unsigned_abs() as u64;
        }
        let is_better_tie =
            energy == best_energy && index.abs_diff(target) < best_index.abs_diff(target);
        if energy < best_energy || is_better_tie {
            best_energy = energy;
            best_index = index;
        }
    }
    best_index
}

pub fn energy(samples: &[i16]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let mut peak = 0i32;
    let mut sum_squares = 0f64;
    for sample in samples {
        let value = *sample as i32;
        peak = peak.max(value.abs());
        sum_squares += f64::from(value) * f64::from(value);
    }
    let peak = (peak as f32 / i16::MAX as f32).min(1.0);
    let rms = ((sum_squares / samples.len() as f64).sqrt() / f64::from(i16::MAX)) as f32;
    (peak, rms.min(1.0))
}

fn validate_header(header: &[u8; WAV_HEADER_BYTES as usize]) -> Result<u64> {
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" || &header[12..16] != b"fmt " {
        bail!("unsupported WAV header");
    }
    let audio_format = u16::from_le_bytes([header[20], header[21]]);
    let channels = u16::from_le_bytes([header[22], header[23]]);
    let sample_rate = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
    let bits_per_sample = u16::from_le_bytes([header[34], header[35]]);
    if audio_format != 1
        || channels != CHANNELS
        || sample_rate != SAMPLE_RATE_HZ
        || bits_per_sample != BITS_PER_SAMPLE
        || &header[36..40] != b"data"
    {
        bail!("expected 16 kHz mono 16-bit PCM WAV");
    }
    Ok(u32::from_le_bytes([header[40], header[41], header[42], header[43]]) as u64)
}

fn audio_data_bytes(file: &mut File, header_data_bytes: u64, file_data_bytes: u64) -> Result<u64> {
    if header_data_bytes == 0 || header_data_bytes > file_data_bytes {
        return Ok(file_data_bytes);
    }
    if header_data_bytes < file_data_bytes
        && !has_known_trailing_chunk(file, header_data_bytes, file_data_bytes - header_data_bytes)?
    {
        return Ok(file_data_bytes);
    }
    Ok(header_data_bytes)
}

fn has_known_trailing_chunk(file: &mut File, offset: u64, remaining: u64) -> Result<bool> {
    if remaining < 8 {
        return Ok(false);
    }

    file.seek(SeekFrom::Start(WAV_HEADER_BYTES + offset))?;
    let mut chunk_header = [0u8; 8];
    file.read_exact(&mut chunk_header)?;
    let chunk_id = &chunk_header[0..4];
    let chunk_len = u32::from_le_bytes([
        chunk_header[4],
        chunk_header[5],
        chunk_header[6],
        chunk_header[7],
    ]) as u64;
    let padded_len = 8 + chunk_len + (chunk_len % 2);

    Ok(is_known_trailing_chunk(chunk_id) && padded_len <= remaining)
}

fn is_known_trailing_chunk(chunk_id: &[u8]) -> bool {
    matches!(
        chunk_id,
        b"LIST" | b"JUNK" | b"bext" | b"fact" | b"cue " | b"smpl" | b"iXML" | b"ID3 "
    )
}

fn write_header(writer: &mut impl Write, data_len: u32) -> Result<()> {
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + data_len).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&CHANNELS.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE_HZ.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE_HZ * BYTES_PER_SAMPLE as u32).to_le_bytes())?;
    writer.write_all(&(BYTES_PER_SAMPLE as u16).to_le_bytes())?;
    writer.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

pub fn ms_to_sample(ms: u64) -> u64 {
    ms.saturating_mul(SAMPLE_RATE_HZ as u64) / 1_000
}

pub fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / SAMPLE_RATE_HZ as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pickscribe-{name}-{id}"))
    }

    #[test]
    fn slice_wav_writes_valid_header_and_expected_data_length() -> Result<()> {
        let dir = temp_dir("wav-slice");
        let source = dir.join("source.wav");
        let destination = dir.join("segment.wav");
        let samples: Vec<i16> = (0..SAMPLE_RATE_HZ as i16).collect();
        write_wav(&source, &samples)?;

        let slice = slice_wav(&source, &destination, 250, 750)?;

        assert_eq!(slice.start_ms, 250);
        assert_eq!(slice.end_ms, 750);
        assert_eq!(slice.sample_count, 8_000);
        let bytes = fs::read(&destination)?;
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            16_000
        );

        let decoded = read_samples(&destination, 0, 500)?;
        assert_eq!(decoded.len(), 8_000);
        assert_eq!(decoded[0], samples[4_000]);
        assert_eq!(decoded[7_999], samples[11_999]);
        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn slice_wav_clamps_to_available_growing_file_data() -> Result<()> {
        let dir = temp_dir("wav-growing");
        let source = dir.join("source.wav");
        let destination = dir.join("segment.wav");
        write_wav(&source, &[1; 1_600])?;

        let slice = slice_wav(&source, &destination, 50, 500)?;

        assert_eq!(slice.start_ms, 50);
        assert_eq!(slice.end_ms, 100);
        assert_eq!(slice.sample_count, 800);
        assert_eq!(read_samples(&destination, 0, 1_000)?.len(), 800);
        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn read_samples_ignores_trailing_non_audio_chunks() -> Result<()> {
        let dir = temp_dir("wav-trailing-chunk");
        let source = dir.join("source.wav");
        write_wav(&source, &[7; 1_600])?;
        let mut file = fs::OpenOptions::new().append(true).open(&source)?;
        file.write_all(b"LIST")?;
        file.write_all(&4u32.to_le_bytes())?;
        file.write_all(b"INFO")?;

        assert_eq!(read_samples(&source, 0, 1_000)?.len(), 1_600);
        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn read_samples_uses_file_length_for_stale_growing_header() -> Result<()> {
        let dir = temp_dir("wav-stale-header");
        let source = dir.join("source.wav");
        write_wav(&source, &[1; 1_600])?;
        let mut file = fs::OpenOptions::new().write(true).open(&source)?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&1_600u32.to_le_bytes())?;

        assert_eq!(read_samples(&source, 0, 1_000)?.len(), 1_600);
        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn low_energy_boundary_prefers_nearby_silence() {
        let mut samples = vec![12_000; 2_000];
        for sample in &mut samples[900..1_100] {
            *sample = 0;
        }

        let boundary = find_low_energy_boundary(&samples, 850, 300);

        assert!((900..=1_100).contains(&boundary), "boundary={boundary}");
    }

    #[test]
    fn low_energy_boundary_keeps_ties_near_target() {
        let samples = vec![0; 2_000];

        assert_eq!(find_low_energy_boundary(&samples, 1_000, 300), 1_000);
    }

    #[test]
    fn energy_reports_peak_and_rms() {
        let (peak, rms) = energy(&[0, i16::MAX, -i16::MAX]);

        assert_eq!(peak, 1.0);
        assert!(rms > 0.8 && rms < 0.9, "rms={rms}");
    }
}
