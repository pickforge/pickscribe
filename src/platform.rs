use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    ShipsNow,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformBlocker {
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformSupport {
    pub os: String,
    pub release_status: ReleaseStatus,
    pub dictation_supported: bool,
    pub summary: String,
    pub blockers: Vec<PlatformBlocker>,
}

impl PlatformSupport {
    pub fn unsupported_dictation_message(&self) -> Option<String> {
        if self.dictation_supported {
            return None;
        }
        Some(format!(
            "{} is not a PickScribe release target yet. Blocking work: {}.",
            self.os_label(),
            self.blockers
                .iter()
                .map(|blocker| blocker.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    pub fn os_label(&self) -> &str {
        match self.os.as_str() {
            "linux" => "Linux",
            "macos" => "macOS",
            "windows" => "Windows",
            _ => "This platform",
        }
    }
}

pub fn current() -> PlatformSupport {
    support_for(std::env::consts::OS)
}

fn support_for(os: &str) -> PlatformSupport {
    match os {
        "linux" => PlatformSupport {
            os: "linux".into(),
            release_status: ReleaseStatus::ShipsNow,
            dictation_supported: true,
            summary: "Linux release target: PipeWire capture, whisper.cpp, Linux clipboard/paste helpers, tray, floating window, and signed updater artifacts.".into(),
            blockers: Vec::new(),
        },
        "macos" => blocked(
            "macos",
            "macOS release is blocked until the native host path is implemented and signed.",
            [
                (
                    "Global shortcuts",
                    "Native shortcut registration and permission flow are missing.",
                ),
                (
                    "Tray/window behavior validation",
                    "Menu bar, dock, main window, and floating window behavior are not validated.",
                ),
                (
                    "Signing/notarization",
                    "Developer ID signing and notarization are not configured.",
                ),
                (
                    "Native-host smoke tests",
                    "Deferred for this PR; required before a real macOS release.",
                ),
            ],
        ),
        "windows" => blocked(
            "windows",
            "Windows release is blocked until the native host path is implemented and code-signed.",
            [
                (
                    "Native audio capture",
                    "WASAPI or Media Foundation microphone capture backend is missing.",
                ),
                (
                    "Paste automation",
                    "Clipboard plus SendInput paste backend is missing.",
                ),
                (
                    "Global shortcuts",
                    "Native shortcut registration and permission behavior are missing.",
                ),
                (
                    "Tray/window behavior validation",
                    "Tray, taskbar, main window, and floating window behavior are not validated.",
                ),
                (
                    "Code signing",
                    "Installer and executable code-signing are not configured.",
                ),
                (
                    "Native-host smoke tests",
                    "Deferred for this PR; required before a real Windows release.",
                ),
            ],
        ),
        other => blocked(
            other,
            "This OS is not a PickScribe release target.",
            [(
                "Platform implementation",
                "No native audio, paste, shortcut, tray/window, signing, or smoke-test plan exists for this OS.",
            )],
        ),
    }
}

fn blocked(
    os: &str,
    summary: &str,
    blockers: impl IntoIterator<Item = (&'static str, &'static str)>,
) -> PlatformSupport {
    PlatformSupport {
        os: os.into(),
        release_status: ReleaseStatus::Blocked,
        dictation_supported: false,
        summary: summary.into(),
        blockers: blockers
            .into_iter()
            .map(|(name, detail)| PlatformBlocker {
                name: name.into(),
                detail: detail.into(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_is_the_only_release_target() {
        let support = support_for("linux");

        assert_eq!(support.release_status, ReleaseStatus::ShipsNow);
        assert!(support.dictation_supported);
        assert!(support.blockers.is_empty());
        assert_eq!(support.unsupported_dictation_message(), None);
    }

    #[test]
    fn macos_release_lists_required_native_work() {
        let support = support_for("macos");
        let blocker_names = support
            .blockers
            .iter()
            .map(|blocker| blocker.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(support.release_status, ReleaseStatus::Blocked);
        assert!(!support.dictation_supported);
        assert!(!blocker_names.contains(&"Native audio capture"));
        assert!(!blocker_names.contains(&"Paste automation"));
        assert!(blocker_names.contains(&"Global shortcuts"));
        assert!(blocker_names.contains(&"Tray/window behavior validation"));
        assert!(blocker_names.contains(&"Signing/notarization"));
        assert!(blocker_names.contains(&"Native-host smoke tests"));
    }

    #[test]
    fn windows_release_lists_required_native_work() {
        let support = support_for("windows");
        let blocker_names = support
            .blockers
            .iter()
            .map(|blocker| blocker.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(support.release_status, ReleaseStatus::Blocked);
        assert!(!support.dictation_supported);
        assert!(blocker_names.contains(&"Native audio capture"));
        assert!(blocker_names.contains(&"Paste automation"));
        assert!(blocker_names.contains(&"Global shortcuts"));
        assert!(blocker_names.contains(&"Tray/window behavior validation"));
        assert!(blocker_names.contains(&"Code signing"));
        assert!(blocker_names.contains(&"Native-host smoke tests"));
    }
}
