import { api } from "./api";

export type ThemeSetting = "system" | "dark" | "light";

let current: ThemeSetting = "system";
let wired = false;

function apply(resolved: "dark" | "light") {
  document.documentElement.dataset.theme = resolved;
}

async function resolveSystem(): Promise<"dark" | "light"> {
  // The backend reads the XDG settings portal — more reliable than the
  // webview's media query under a forced X11 backend.
  try {
    const theme = await api.getSystemTheme();
    if (theme === "light" || theme === "dark") return theme;
  } catch {
    // fall through to the media query
  }
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

export async function setTheme(setting: ThemeSetting): Promise<void> {
  current = setting;
  apply(setting === "system" ? await resolveSystem() : setting);
}

export function initTheme(initial: ThemeSetting): void {
  if (!wired) {
    wired = true;
    window
      .matchMedia?.("(prefers-color-scheme: light)")
      ?.addEventListener("change", () => {
        if (current === "system") void setTheme("system");
      });
    // Re-check when the user comes back: KDE theme switches don't always
    // reach the webview as a media-query change.
    window.addEventListener("focus", () => {
      if (current === "system") void setTheme("system");
    });
  }
  void setTheme(initial);
}
