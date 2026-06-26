<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    api,
    desktopApiAvailable,
    EVENT_LEVEL,
    EVENT_STATE,
    type StatePayload,
    type Stage,
  } from "./lib/api";
  import Waveform from "./lib/components/Waveform.svelte";

  const LEVEL_BARS = 24;

  let stage = $state<Stage>("idle");
  let levels = $state<number[]>(Array(LEVEL_BARS).fill(0));

  onMount(() => {
    if (!desktopApiAvailable()) {
      return;
    }

    const unsubs: Array<() => void> = [];
    api.getState().then((s) => (stage = s.stage)).catch(() => {});
    listen<StatePayload>(EVENT_STATE, (event) => {
      stage = event.payload.stage;
      if (event.payload.stage === "idle") {
        levels = Array(LEVEL_BARS).fill(0);
      }
    }).then((u) => unsubs.push(u));
    listen<number>(EVENT_LEVEL, (event) => {
      levels = [...levels.slice(1), event.payload];
    }).then((u) => unsubs.push(u));
    return () => unsubs.forEach((u) => u());
  });

  const busy = $derived(stage === "transcribing" || stage === "cleaning" || stage === "pasting");

  // Distinguish click (open the app) from drag (move the window): once the
  // pointer travels past a small threshold, hand control to the compositor.
  let downAt: { x: number; y: number } | null = null;
  let dragged = false;

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    downAt = { x: event.screenX, y: event.screenY };
    dragged = false;
  }

  function onPointerMove(event: PointerEvent) {
    if (!downAt || dragged) return;
    const dx = event.screenX - downAt.x;
    const dy = event.screenY - downAt.y;
    if (Math.hypot(dx, dy) > 5) {
      dragged = true;
      getCurrentWindow().startDragging().catch(() => {});
    }
  }

  function onPointerUp(event: PointerEvent) {
    if (event.button === 0 && downAt && !dragged) {
      api.showMainWindow().catch(() => {});
    } else if (event.button === 1) {
      // Middle-click dismisses the capsule (persisted; re-enable from the
      // tray menu or Settings).
      api.toggleFloatButton().catch(() => {});
    }
    downAt = null;
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault();
    api.toggleDictation().catch(() => {});
  }
</script>

<div
  class="capsule"
  class:recording={stage === "recording"}
  class:busy
  role="button"
  tabindex="-1"
  aria-label="PickScribe — click to open, right-click to toggle dictation, middle-click to hide"
  title="PickScribe — click to open, right-click to toggle dictation, middle-click to hide, drag to move"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  oncontextmenu={onContextMenu}
>
  <img class="mark mark-dark" src="/brand/pickscribe-mark-128.svg" alt="" draggable="false" />
  <img class="mark mark-light" src="/brand/pickscribe-mark-light.svg" alt="" draggable="false" />

  <div class="wave">
    <Waveform {levels} active={stage === "recording"} height={28} />
  </div>

  {#if busy}
    <span class="spinner"></span>
  {:else}
    <span class="status-dot" class:pulse={stage === "recording"}></span>
  {/if}
</div>

<style>
  .capsule {
    display: flex;
    align-items: center;
    gap: 10px;
    width: calc(100vw - 4px);
    height: calc(100vh - 4px);
    margin: 2px;
    padding: 0 14px 0 10px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-pill);
    background: var(--capsule-bg);
    backdrop-filter: blur(12px) saturate(140%);
    box-shadow: var(--glow-ember-soft);
    cursor: pointer;
    user-select: none;
    -webkit-user-select: none;
    overflow: hidden;
    transition: border-color 500ms var(--ease-forge), box-shadow 500ms var(--ease-forge);
  }
  .capsule:hover {
    border-color: color-mix(in srgb, var(--ember) 40%, transparent);
  }
  .capsule.recording {
    border-color: color-mix(in srgb, var(--ember) 60%, transparent);
    box-shadow: var(--glow-ember);
  }

  .mark {
    flex: none;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    pointer-events: none;
  }
  .mark-light {
    display: none;
  }
  :global([data-theme="light"]) .mark-dark {
    display: none;
  }
  :global([data-theme="light"]) .mark-light {
    display: block;
  }

  .wave {
    flex: 1;
    min-width: 0;
    pointer-events: none;
  }

  .status-dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--muted);
    transition: background 300ms var(--ease-forge);
  }
  .capsule.recording .status-dot {
    background: var(--ember);
  }

  .status-dot.pulse {
    animation: ember-pulse 2.4s var(--ease-forge) infinite;
  }

  .spinner {
    flex: none;
    width: 13px;
    height: 13px;
    border-radius: 999px;
    border: 2px solid color-mix(in srgb, var(--ember) 25%, transparent);
    border-top-color: var(--ember);
    animation: spin 0.9s linear infinite;
  }
</style>
