//! Embeds the short commit, so the bar widget and auto-update builds show what is running.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PICKSCRIBE_COMMIT");
    let commit = std::env::var("PICKSCRIBE_COMMIT").ok().or_else(git_commit);
    println!(
        "cargo:rustc-env=PICKSCRIBE_COMMIT={}",
        commit.as_deref().unwrap_or("unknown")
    );
}

fn git_commit() -> Option<String> {
    // The HEAD reflog changes on every commit and checkout in this worktree.
    if let Some(reflog) = git(&["rev-parse", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={reflog}");
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"])?;
    let dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .is_ok_and(|status| !status.success());
    Some(if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    })
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
