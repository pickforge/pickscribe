const INTERACTIVE_TITLEBAR_SELECTOR =
  "button, a, input, select, textarea, [role='button'], [contenteditable='true']";

export function handleTitlebarMouseDown(event: MouseEvent): void {
  const target = event.target as { closest?: (selector: string) => Element | null } | null;
  if (event.button !== 0 || target?.closest?.(INTERACTIVE_TITLEBAR_SELECTOR)) {
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  const doubleClick = event.detail === 2;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      return doubleClick ? win.toggleMaximize() : win.startDragging();
    })
    .catch(() => {});
}
