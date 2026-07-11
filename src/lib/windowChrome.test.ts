import { beforeEach, describe, expect, it, vi } from "vitest";

const toggleMaximize = vi.fn(() => Promise.resolve());
const startDragging = vi.fn(() => Promise.resolve());

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging, toggleMaximize }),
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
  startDragging.mockClear();
});

describe("handleTitlebarMouseDown", () => {
  it("toggles maximize for a primary-button double click", async () => {
    const event = mouseDown();
    handleTitlebarMouseDown(event);

    await vi.waitFor(() => expect(toggleMaximize).toHaveBeenCalledOnce());
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

  it("starts window dragging for a single press", async () => {
    const event = mouseDown(1);
    handleTitlebarMouseDown(event);

    await vi.waitFor(() => expect(startDragging).toHaveBeenCalledOnce());
    expect(toggleMaximize).not.toHaveBeenCalled();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
  });
});
