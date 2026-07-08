import { afterEach, describe, expect, it, vi } from "vitest";

async function loadHostPlatform() {
  vi.resetModules();
  const mod = await import("./platform");
  return mod.hostPlatform;
}

describe("hostPlatform", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("is web outside the Tauri webview", async () => {
    const hostPlatform = await loadHostPlatform();
    expect(hostPlatform()).toBe("web");
  });

  it("detects macOS from the user agent", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    });
    const hostPlatform = await loadHostPlatform();
    expect(hostPlatform()).toBe("macos");
  });

  it("detects Windows from the user agent", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" });
    const hostPlatform = await loadHostPlatform();
    expect(hostPlatform()).toBe("windows");
  });

  it("falls back to Linux and caches the result", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (X11; Linux x86_64)" });
    const hostPlatform = await loadHostPlatform();
    expect(hostPlatform()).toBe("linux");
    expect(hostPlatform()).toBe("linux");
  });
});
