import {
  createEligibility,
  createProcessCheckGate,
  createUpdateController,
  type UpdateAdapter,
} from "@pickforge/tauri-updater";
import { describe, expect, it, vi } from "vitest";
import { studioUpdateDialogEnabled } from "./flags";
import { scheduleStartupUpdate, type UpdateWindow } from "./updateStartup";

class FakeUpdateWindow implements UpdateWindow {
  visible = true;
  focused = true;
  listener?: (event: { payload: boolean }) => void;
  onRegister?: () => void;

  async isVisible() {
    return this.visible;
  }

  async isFocused() {
    return this.focused;
  }

  async onFocusChanged(listener: (event: { payload: boolean }) => void) {
    this.listener = listener;
    this.onRegister?.();
    return () => {
      this.listener = undefined;
    };
  }

  emitFocus(focused: boolean) {
    this.focused = focused;
    this.listener?.({ payload: focused });
  }
}

function injectedController(check = vi.fn(async () => null)) {
  const adapter: UpdateAdapter = {
    check,
    async downloadAndInstall() {},
    async relaunch() {},
  };
  return {
    check,
    controller: createUpdateController({
      adapter,
      gate: createProcessCheckGate(),
      eligibility: createEligibility({
        packaged: true,
        mainWindow: true,
        visible: true,
        focused: true,
      }),
    }),
  };
}

async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("startup updater integration", () => {
  it("declares the shared dialog flag default off", () => {
    expect(studioUpdateDialogEnabled()).toBe(false);
  });

  it("keeps the legacy updater selected while the flag is off", async () => {
    const window = new FakeUpdateWindow();
    const legacyCheck = vi.fn(async () => {});
    const studio = injectedController();

    await scheduleStartupUpdate({
      studioEnabled: false,
      window,
      legacyCheck,
      studioController: studio.controller,
    });
    await flush();

    expect(legacyCheck).toHaveBeenCalledOnce();
    expect(studio.check).not.toHaveBeenCalled();
  });

  it("selects the injected shared controller when the flag is on", async () => {
    const window = new FakeUpdateWindow();
    const legacyCheck = vi.fn(async () => {});
    const studio = injectedController();

    await scheduleStartupUpdate({
      studioEnabled: true,
      window,
      legacyCheck,
      studioController: studio.controller,
    });
    await flush();

    expect(studio.check).toHaveBeenCalledOnce();
    expect(legacyCheck).not.toHaveBeenCalled();
  });

  it("defers the shared check until a visible main window is focused", async () => {
    const window = new FakeUpdateWindow();
    window.focused = false;
    const studio = injectedController();

    await scheduleStartupUpdate({
      studioEnabled: true,
      window,
      legacyCheck: vi.fn(async () => {}),
      studioController: studio.controller,
    });
    await flush();
    expect(studio.check).not.toHaveBeenCalled();

    window.emitFocus(true);
    await flush();
    expect(studio.check).toHaveBeenCalledOnce();
  });

  it("does not miss eligibility reached while the focus listener registers", async () => {
    const window = new FakeUpdateWindow();
    window.visible = false;
    window.focused = false;
    window.onRegister = () => {
      window.visible = true;
      window.focused = true;
    };
    const studio = injectedController();

    await scheduleStartupUpdate({
      studioEnabled: true,
      window,
      legacyCheck: vi.fn(async () => {}),
      studioController: studio.controller,
    });
    await flush();

    expect(studio.check).toHaveBeenCalledOnce();
  });

  it("excludes hidden autostart until first focus and checks only once", async () => {
    const window = new FakeUpdateWindow();
    window.visible = false;
    window.focused = false;
    const studio = injectedController();

    await scheduleStartupUpdate({
      studioEnabled: true,
      window,
      legacyCheck: vi.fn(async () => {}),
      studioController: studio.controller,
    });
    window.emitFocus(false);
    await flush();
    expect(studio.check).not.toHaveBeenCalled();

    window.visible = true;
    window.emitFocus(true);
    window.emitFocus(true);
    await flush();
    expect(studio.check).toHaveBeenCalledOnce();
  });
});
