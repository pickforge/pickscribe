import { describe, expect, it } from "vitest";

import { settingsPlatformDisplayState, settingsSaveDisplayState } from "./settingsDisplay";

describe("settingsSaveDisplayState", () => {
  it("shows a visible, disabled header Save action while clean and no overlay", () => {
    expect(settingsSaveDisplayState(false)).toEqual({
      headerSaveHidden: false,
      headerSaveDisabled: true,
      overlayVisible: false,
    });
  });

  it("hides the header Save action and shows the overlay while dirty", () => {
    expect(settingsSaveDisplayState(true)).toEqual({
      headerSaveHidden: true,
      headerSaveDisabled: false,
      overlayVisible: true,
    });
  });

  it("never presents both the header action and the overlay at once", () => {
    for (const dirty of [false, true]) {
      const state = settingsSaveDisplayState(dirty);
      const headerPresented = !state.headerSaveHidden;
      expect(headerPresented && state.overlayVisible).toBe(false);
    }
  });
});

describe("settingsPlatformDisplayState", () => {
  it("shows the in-app shortcut only on macOS", () => {
    expect(settingsPlatformDisplayState("macos")).toEqual({
      shortcutFieldVisible: true,
      desktopKeybindingHelpVisible: false,
    });
    expect(settingsPlatformDisplayState("linux")).toEqual({
      shortcutFieldVisible: false,
      desktopKeybindingHelpVisible: true,
    });
    expect(settingsPlatformDisplayState("windows")).toEqual({
      shortcutFieldVisible: false,
      desktopKeybindingHelpVisible: false,
    });
    expect(settingsPlatformDisplayState("web")).toEqual({
      shortcutFieldVisible: false,
      desktopKeybindingHelpVisible: false,
    });
  });
});
