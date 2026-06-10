pub mod cleanup;
pub mod levels;
pub mod paste;
pub mod recorder;
pub mod sounds;
pub mod stt;

use std::path::PathBuf;

pub fn find_command(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn command_exists(name: &str) -> bool {
    find_command(name).is_some()
}
