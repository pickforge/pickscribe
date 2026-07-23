import type { UpdateAdapter } from "@pickforge/tauri-updater";
import { mountStudioUpdater } from "./studioUpdater";

const fixtureUpdate = {
  version: "0.3.0",
  notes: "Faster local transcription\nImproved floating capsule behavior\nMore reliable update delivery",
};

export async function mountUpdateFixture(state: "available" | "downloading"): Promise<void> {
  const adapter: UpdateAdapter = {
    async check() {
      return fixtureUpdate;
    },
    async downloadAndInstall(onEvent) {
      onEvent({ type: "started", contentLength: 100 });
      onEvent({ type: "progress", chunkLength: 62 });
      await new Promise<void>(() => {});
    },
    async relaunch() {},
  };

  const mounted = mountStudioUpdater({
    adapter,
    eligibility: { packaged: true, mainWindow: true, visible: true, focused: true },
    metadata: { productName: "PickScribe", productMark: "PS", currentVersion: "0.2.0" },
  });

  const controller = mounted.controller;
  await controller.check({ manual: true });
  if (state === "downloading") void controller.install();
}

export function fixtureStateFromLocation(): "available" | "downloading" | null {
  if (!import.meta.env.DEV) return null;
  const state = new URLSearchParams(window.location.search).get("update-fixture");
  return state === "available" || state === "downloading" ? state : null;
}
