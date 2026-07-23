import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  createEligibility,
  createTauriUpdaterAdapter,
  createUpdateController,
  definePickforgeUpdaterElement,
  type StaticEligibility,
  type UpdateAdapter,
  type UpdateController,
  type UpdateDialogMetadata,
} from "@pickforge/tauri-updater";
import packageJson from "../../package.json";

const defaultMetadata: UpdateDialogMetadata = {
  productName: "PickScribe",
  productMark: "PS",
  currentVersion: packageJson.version,
};

interface StudioUpdaterOptions {
  adapter?: UpdateAdapter;
  eligibility?: StaticEligibility;
  metadata?: UpdateDialogMetadata;
  host?: HTMLElement;
}

export interface MountedStudioUpdater {
  controller: UpdateController;
  destroy(): void;
}

export function mountStudioUpdater(options: StudioUpdaterOptions = {}): MountedStudioUpdater {
  const controller = createUpdateController({
    adapter:
      options.adapter ??
      createTauriUpdaterAdapter({
        check,
        relaunch,
      }),
    // Static eligibility: visible/focused deferral is NOT handled here — the
    // caller must gate `controller.start()` through `scheduleStartupUpdate`,
    // which owns the visible+focused main-window wait for this app.
    eligibility: createEligibility(
      options.eligibility ?? {
        packaged: !import.meta.env.DEV,
        mainWindow: true,
        visible: true,
        focused: true,
      },
    ),
  });

  definePickforgeUpdaterElement();
  const dialog = document.createElement("pickforge-update-dialog") as HTMLElement & {
    controller: UpdateController;
    metadata: UpdateDialogMetadata;
  };
  dialog.metadata = options.metadata ?? defaultMetadata;
  dialog.controller = controller;
  (options.host ?? document.body).append(dialog);

  return {
    controller,
    destroy() {
      dialog.remove();
    },
  };
}
