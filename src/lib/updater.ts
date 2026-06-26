// In-app auto-update over the Tauri updater plugin. The plugin polls the GitHub
// `latest.json` endpoint from tauri.conf.json and verifies the signed bundle
// against the embedded public key. No-ops in dev and in browser preview.
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { desktopApiAvailable } from "./api";

/** Check once on startup; if an update exists, ask the user, then install + relaunch. */
export async function checkForUpdates(): Promise<void> {
  // `tauri dev` serves the released latest.json, which would read as a no-op or
  // a downgrade — only run inside a packaged build.
  if (import.meta.env.DEV || !desktopApiAvailable()) {
    return;
  }

  try {
    const update = await check();
    if (!update) {
      return;
    }

    const notes = update.body ? `\n\n${update.body}` : "";
    const proceed = window.confirm(
      `PickScribe ${update.version} is available.${notes}\n\nDownload and install now? The app will restart.`,
    );
    if (!proceed) {
      return;
    }

    await update.downloadAndInstall();
    await relaunch();
  } catch (err) {
    // Network blips / retracted releases shouldn't disrupt startup.
    console.error("Update check failed:", err);
  }
}
