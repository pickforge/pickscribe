import { mount } from "svelte";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import "@fontsource/geist-sans/400.css";
import "@fontsource/geist-sans/500.css";
import "@fontsource/geist-sans/600.css";
import "@fontsource/geist-sans/700.css";
import "@fontsource/geist-mono/400.css";
import "@fontsource/geist-mono/500.css";
import "@fontsource/geist-mono/600.css";
import "./app.css";
import App from "./App.svelte";
import Float from "./Float.svelte";
import { api, EVENT_CONFIG, type AppConfig } from "./lib/api";
import { initTheme, setTheme, type ThemeSetting } from "./lib/theme";

const isFloat = getCurrentWindow().label === "float";
document.documentElement.classList.toggle("is-float", isFloat);

api
  .getConfig()
  .then((config) => initTheme(config.general.theme as ThemeSetting))
  .catch(() => initTheme("system"));
void listen<AppConfig>(EVENT_CONFIG, (event) => {
  void setTheme(event.payload.general.theme as ThemeSetting);
});

const target = document.getElementById("app");
if (!target) throw new Error("App root element was not found");

export default mount(isFloat ? Float : App, { target });
