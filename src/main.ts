import "@fontsource/geist-sans/400.css";
import "@fontsource/geist-sans/500.css";
import "@fontsource/geist-sans/600.css";
import "@fontsource/geist-sans/700.css";
import "@fontsource/geist-mono/400.css";
import "@fontsource/geist-mono/500.css";
import "@fontsource/geist-mono/600.css";
import { listen } from "@tauri-apps/api/event";
import { mount } from "svelte";
import App from "./App.svelte";
import Float from "./Float.svelte";
import { api, desktopApiAvailable, EVENT_CONFIG, type AppConfig } from "./lib/api";
import { initTheme, setTheme, type ThemeSetting } from "./lib/theme";
import "./app.css";

const target = document.getElementById("app");

if (!target) {
  throw new Error("App root element was not found");
}

function currentWindowLabel() {
  const internals = (
    window as Window & {
      __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
    }
  ).__TAURI_INTERNALS__;

  if (internals?.metadata?.currentWindow?.label) {
    return internals.metadata.currentWindow.label;
  }

  // Browser preview only: ?window=float renders the floating capsule.
  return new URLSearchParams(window.location.search).get("window") ?? "main";
}

const component = currentWindowLabel() === "float" ? Float : App;

if (component === Float) {
  document.documentElement.classList.add("is-float");
  document.body.classList.add("float-host");
}

if (desktopApiAvailable()) {
  api
    .getAppConfig()
    .then((config) => initTheme(config.general.theme as ThemeSetting))
    .catch(() => initTheme("system"));
  void listen<AppConfig>(EVENT_CONFIG, (event) => {
    void setTheme(event.payload.general.theme as ThemeSetting);
  });
} else {
  initTheme("system");
}

export default mount(component, { target });
