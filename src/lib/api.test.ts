import { afterEach, describe, expect, it, vi } from "vitest";

import { desktopApiAvailable, formatDuration, formatError, formatMinutes } from "./api";

describe("desktopApiAvailable", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("is false outside the Tauri webview", () => {
    expect(desktopApiAvailable()).toBe(false);
  });

  it("is true when Tauri internals are present", () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    expect(desktopApiAvailable()).toBe(true);
  });
});

describe("formatDuration", () => {
  it("formats seconds and minutes", () => {
    expect(formatDuration(4200)).toBe("4s");
    expect(formatDuration(65_000)).toBe("1m 05s");
  });
});

describe("formatMinutes", () => {
  it("formats saved time units", () => {
    expect(formatMinutes(0.4)).toBe("24s");
    expect(formatMinutes(12.4)).toBe("12m");
    expect(formatMinutes(125)).toBe("2h 05m");
  });
});

describe("formatError", () => {
  it("preserves useful error messages", () => {
    expect(formatError("plain")).toBe("plain");
    expect(formatError(new Error("boom"))).toBe("boom");
    expect(formatError({ code: "E_PICKSCRIBE" })).toBe('{"code":"E_PICKSCRIBE"}');
  });
});
