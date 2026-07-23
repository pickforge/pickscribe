import type { UpdateController } from "@pickforge/tauri-updater";

export interface UpdateWindow {
  isVisible(): Promise<boolean>;
  isFocused(): Promise<boolean>;
  onFocusChanged(listener: (event: { payload: boolean }) => void): Promise<() => void>;
}

interface StartupUpdateOptions {
  studioEnabled: boolean;
  window: UpdateWindow;
  legacyCheck: () => Promise<void>;
  studioController?: UpdateController;
}

const noop = () => {};

export async function scheduleStartupUpdate({
  studioEnabled,
  window,
  legacyCheck,
  studioController,
}: StartupUpdateOptions): Promise<() => void> {
  const check = studioEnabled ? () => studioController?.start() ?? Promise.resolve() : legacyCheck;
  let done = false;

  const runOnce = () => {
    if (done) return;
    done = true;
    void check().catch(noop);
  };

  try {
    const unsubscribe = await window.onFocusChanged(({ payload: focused }) => {
      if (!focused) return;
      if (!studioEnabled) {
        runOnce();
        return;
      }
      void Promise.all([window.isVisible(), window.isFocused()])
        .then(([isVisible, isFocused]) => {
          if (isVisible && isFocused) runOnce();
        })
        .catch(noop);
    });

    const visible = await window.isVisible();
    if (visible && (!studioEnabled || (await window.isFocused()))) runOnce();
    return unsubscribe;
  } catch {
    return noop;
  }
}
