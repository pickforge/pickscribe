import { describe, expect, it } from "vitest";

import {
  mergeExternalSettings,
  reconcileExternalSettings,
  shouldApplySaveResponse,
} from "./settingsMerge";

describe("mergeExternalSettings", () => {
  const baseline = {
    general: { sounds: true, float_button: true },
    cleanup: { provider: "auto", model: "" },
  };

  it("refreshes fields that are still clean", () => {
    const external = {
      general: { sounds: true, float_button: false },
      cleanup: { provider: "auto", model: "" },
    };

    expect(mergeExternalSettings(baseline, baseline, external)).toEqual(external);
  });

  it("keeps dirty fields while refreshing unrelated external changes", () => {
    const local = {
      general: { sounds: false, float_button: true },
      cleanup: { provider: "auto", model: "" },
    };
    const external = {
      general: { sounds: true, float_button: false },
      cleanup: { provider: "ollama", model: "qwen" },
    };

    expect(mergeExternalSettings(baseline, local, external)).toEqual({
      general: { sounds: false, float_button: false },
      cleanup: { provider: "ollama", model: "qwen" },
    });
  });

  it("keeps the local value when both sides changed the same field", () => {
    const local = {
      general: { sounds: true, float_button: true },
      cleanup: { provider: "ollama", model: "qwen" },
    };
    const external = {
      general: { sounds: true, float_button: true },
      cleanup: { provider: "openai", model: "gpt" },
    };

    expect(mergeExternalSettings(baseline, local, external).cleanup).toEqual({
      provider: "ollama",
      model: "qwen",
    });
  });

  it("advances the discard baseline while preserving dirty local fields", () => {
    const local = {
      general: { sounds: false, float_button: true },
      cleanup: { provider: "auto", model: "" },
    };
    const external = {
      general: { sounds: true, float_button: false },
      cleanup: { provider: "openai", model: "gpt" },
    };

    const resolution = reconcileExternalSettings(baseline, local, external);

    expect(resolution).toEqual({
      config: {
        general: { sounds: false, float_button: false },
        cleanup: { provider: "openai", model: "gpt" },
      },
      baseline: external,
      keptLocalChanges: true,
    });
  });

  it("adopts a clean external update without showing a conflict", () => {
    const external = {
      general: { sounds: true, float_button: false },
      cleanup: { provider: "auto", model: "" },
    };

    expect(reconcileExternalSettings(baseline, baseline, external)).toEqual({
      config: external,
      baseline: external,
      keptLocalChanges: false,
    });
  });

  it("preserves a local shortcut edit while merging unrelated external changes", () => {
    const shortcutBaseline = {
      general: { sounds: true },
      shortcut: { toggle: "Cmd+Shift+Space" },
    };
    const local = {
      general: { sounds: true },
      shortcut: { toggle: "Cmd+Option+D" },
    };
    const external = {
      general: { sounds: false },
      shortcut: { toggle: "Cmd+Shift+Space" },
    };

    expect(mergeExternalSettings(shortcutBaseline, local, external)).toEqual({
      general: { sounds: false },
      shortcut: { toggle: "Cmd+Option+D" },
    });
  });

  it("ignores a save response after any newer config event", () => {
    expect(shouldApplySaveResponse(4, 4)).toBe(true);
    expect(shouldApplySaveResponse(4, 5)).toBe(false);
  });
});
