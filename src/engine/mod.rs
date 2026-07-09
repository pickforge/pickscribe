pub mod audio_segments;
pub mod cleanup;
pub mod incremental;
pub mod levels;
pub mod media;
pub mod paste;
pub mod recorder;
pub mod segments;
pub mod sounds;
pub mod stt;
pub mod transcript;

use std::path::PathBuf;

pub fn find_command(name: &str) -> Option<PathBuf> {
    // Fall back to ~/.local/bin: setup installs whisper-cli there, but GUI
    // sessions launched from the app menu don't always have it on PATH.
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    find_in_dirs(name, &dirs)
}

fn find_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn command_exists(name: &str) -> bool {
    find_command(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pickscribe-{name}-{id}"))
    }

    #[test]
    fn find_in_dirs_finds_file_in_later_dir() {
        let empty = temp_dir("find-empty");
        let bin = temp_dir("find-bin");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("whisper-cli");
        std::fs::write(&exe, b"").unwrap();

        let dirs = vec![empty.clone(), bin.clone()];
        assert_eq!(find_in_dirs("whisper-cli", &dirs), Some(exe));
        assert_eq!(find_in_dirs("missing-cmd", &dirs), None);

        std::fs::remove_dir_all(empty).ok();
        std::fs::remove_dir_all(bin).ok();
    }
}
