//! macOS-only Accessibility permission probing.
//!
//! Paste delivery on macOS synthesizes keystrokes via `osascript`/System
//! Events, which macOS only allows once the user has granted PickScribe
//! Accessibility access. Doctor uses this to detect and explain that
//! requirement before the user hits a confusing runtime failure.

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// Returns whether this process has been granted Accessibility permission
/// (System Settings -> Privacy & Security -> Accessibility). Required for
/// `osascript`-driven paste/type delivery to work.
#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    true
}
