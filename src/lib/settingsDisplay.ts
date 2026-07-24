import type { HostPlatform } from "./platform";

export type SettingsSaveDisplayState = {
  headerSaveHidden: boolean;
  headerSaveDisabled: boolean;
  overlayVisible: boolean;
};

// Single source of truth for where the primary Settings save action lives.
// Clean: the header Save button is visible and disabled. Dirty: the header
// Save button is hidden (without shifting layout) and a viewport-level
// overlay owns the save/discard action instead. Exactly one is ever
// presented at a time.
export function settingsSaveDisplayState(dirty: boolean): SettingsSaveDisplayState {
  return {
    headerSaveHidden: dirty,
    headerSaveDisabled: !dirty,
    overlayVisible: dirty,
  };
}

export function settingsPlatformDisplayState(platform: HostPlatform) {
  return {
    shortcutFieldVisible: platform === "macos",
    desktopKeybindingHelpVisible: platform === "linux",
  };
}
