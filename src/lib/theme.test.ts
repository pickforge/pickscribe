import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "./api";
import { setTheme } from "./theme";

describe("setTheme", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("does not let a late system resolution overwrite a newer theme", async () => {
    let resolveSystem!: (theme: string) => void;
    vi.spyOn(api, "getSystemTheme").mockReturnValue(
      new Promise<string>((resolve) => {
        resolveSystem = resolve;
      })
    );
    const dataset: Record<string, string> = {};
    vi.stubGlobal("document", { documentElement: { dataset } });

    const pendingSystem = setTheme("system");
    await setTheme("dark");
    resolveSystem("light");
    await pendingSystem;

    expect(dataset.theme).toBe("dark");
  });
});
