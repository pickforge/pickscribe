import { beforeEach, describe, expect, it, vi } from "vitest";

const toggleMaximize = vi.fn(() => Promise.resolve());

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ toggleMaximize }),
}));

import { handleTitlebarMouseDown } from "./windowChrome";

function mouseDown(detail = 2, button = 0, interactive = false): MouseEvent {
  return {
    button,
    detail,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
    target: {
      closest: vi.fn(() => (interactive ? {} : null)),
    },
  } as unknown as MouseEvent;
}

beforeEach(() => {
  toggleMaximize.mockClear();
});

describe("handleTitlebarMouseDown", () => {
  it("toggles maximize for a primary-button double click", () => {
    const event = mouseDown();
    handleTitlebarMouseDown(event);

    expect(toggleMaximize).toHaveBeenCalledOnce();
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
  });

  it("ignores interactive titlebar children", () => {
    handleTitlebarMouseDown(mouseDown(2, 0, true));

    expect(toggleMaximize).not.toHaveBeenCalled();
  });

  it("ignores non-primary double clicks", () => {
    handleTitlebarMouseDown(mouseDown(2, 2));

    expect(toggleMaximize).not.toHaveBeenCalled();
  });

  it("leaves a single press available for window dragging", () => {
    const event = mouseDown(1);
    handleTitlebarMouseDown(event);

    expect(toggleMaximize).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();
  });
});
