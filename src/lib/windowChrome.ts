import { getCurrentWindow } from "@tauri-apps/api/window";

const INTERACTIVE_TITLEBAR_SELECTOR =
  "button, a, input, select, textarea, [role='button'], [contenteditable='true']";

export function handleTitlebarMouseDown(event: MouseEvent): void {
  const target = event.target as { closest?: (selector: string) => Element | null } | null;
  if (
    event.button !== 0 ||
    event.detail !== 2 ||
    target?.closest?.(INTERACTIVE_TITLEBAR_SELECTOR)
  ) {
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  void getCurrentWindow()
    .toggleMaximize()
    .catch(() => {});
}
