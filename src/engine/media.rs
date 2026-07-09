use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const MEDIA_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "m4a", "aac", "ogg", "opus", "flac", "wma", "mp4", "mkv", "mov", "webm",
    "avi", "m4v",
];

pub fn resolve_ffmpeg() -> Result<PathBuf> {
    super::find_command("ffmpeg")
        .context("ffmpeg not found in PATH — install ffmpeg to transcribe media files")
}

pub fn convert_to_wav_16k_mono(
    input: &Path,
    output: &Path,
    is_cancelled: impl Fn() -> bool,
) -> Result<()> {
    let ffmpeg = resolve_ffmpeg()?;
    let stderr_path = log_path(output);
    let stderr_file = fs::File::create(&stderr_path)
        .with_context(|| format!("creating ffmpeg log {}", stderr_path.display()))?;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-y", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file));
    let mut child = match cmd.spawn().context("running ffmpeg") {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_file(&stderr_path);
            return Err(err);
        }
    };
    let status: ExitStatus;
    loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(output);
            let _ = fs::remove_file(&stderr_path);
            bail!("conversion cancelled");
        }
        if let Some(exit_status) = child.try_wait()? {
            status = exit_status;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !status.success() {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = fs::remove_file(output);
        let _ = fs::remove_file(&stderr_path);
        bail!("ffmpeg failed: {}", last_lines(&stderr, 15));
    }

    let _ = fs::remove_file(&stderr_path);
    Ok(())
}

pub fn wav_duration_ms(path: &Path) -> Result<i64> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut riff_header = [0u8; 12];
    file.read_exact(&mut riff_header)
        .with_context(|| format!("reading WAV header from {}", path.display()))?;
    if &riff_header[0..4] != b"RIFF" || &riff_header[8..12] != b"WAVE" {
        bail!("unsupported WAV header");
    }

    let mut format = None;
    let mut data_bytes = None;
    loop {
        let mut chunk_header = [0u8; 8];
        file.read_exact(&mut chunk_header)
            .context("reading WAV chunk header")?;
        let chunk_len = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as u64;

        if &chunk_header[0..4] == b"fmt " {
            if chunk_len < 16 {
                bail!("unsupported WAV header");
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt).context("reading WAV format chunk")?;
            format = Some((
                u16::from_le_bytes([fmt[0], fmt[1]]),
                u16::from_le_bytes([fmt[2], fmt[3]]),
                u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]),
                u16::from_le_bytes([fmt[14], fmt[15]]),
            ));
            skip_chunk(&mut file, chunk_len - 16)?;
        } else {
            if &chunk_header[0..4] == b"data" {
                data_bytes = Some(chunk_len);
            }
            skip_chunk(&mut file, chunk_len)?;
        }

        if format.is_some() && data_bytes.is_some() {
            break;
        }
    }

    let (audio_format, channels, sample_rate, bits_per_sample) = format.context("missing WAV format chunk")?;
    if audio_format != 1 || channels != 1 || sample_rate != 16_000 || bits_per_sample != 16 {
        bail!("expected 16 kHz mono 16-bit PCM WAV");
    }
    let data_bytes = data_bytes.context("missing WAV data chunk")?;
    Ok((data_bytes as i64 * 1_000) / 32_000)
}

fn log_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".ffmpeg.log");
    PathBuf::from(path)
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<_> = text.lines().rev().take(count).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn skip_chunk(file: &mut fs::File, chunk_len: u64) -> Result<()> {
    let padded_len = chunk_len + chunk_len % 2;
    file.seek(SeekFrom::Current(padded_len as i64))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::audio_segments::{SAMPLE_RATE_HZ, write_wav};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pickscribe-{name}-{id}"))
    }

    #[test]
    fn wav_duration_reads_16khz_mono_pcm_data() -> Result<()> {
        let dir = temp_dir("media-duration");
        let path = dir.join("audio.wav");
        write_wav(&path, &vec![0; SAMPLE_RATE_HZ as usize * 3 / 2])?;

        assert_eq!(wav_duration_ms(&path)?, 1_500);

        fs::remove_dir_all(dir).ok();
        Ok(())
    }
}
