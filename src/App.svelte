<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Microphone from "phosphor-svelte/lib/Microphone";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import GearSix from "phosphor-svelte/lib/GearSix";
  import {
    api,
    EVENT_HISTORY,
    EVENT_LEVEL,
    EVENT_STATE,
    type StatePayload,
  } from "./lib/api";
  import Dashboard from "./lib/views/Dashboard.svelte";
  import History from "./lib/views/History.svelte";
  import Settings from "./lib/views/Settings.svelte";

  type View = "dashboard" | "history" | "settings";

  const LEVEL_BARS = 56;

  let view = $state<View>("dashboard");
  let settingsDirty = $state(false);
  let pendingView = $state<View | null>(null);
  let settingsActions: { save: () => Promise<boolean>; discard: () => void } | null = null;

  function navigate(target: View) {
    if (view === "settings" && settingsDirty && target !== "settings") {
      pendingView = target;
      return;
    }
    view = target;
  }

  async function saveAndContinue() {
    if (!settingsActions || !pendingView) return;
    if (await settingsActions.save()) {
      view = pendingView;
      pendingView = null;
    } else {
      // Save failed; stay on settings so the error is visible.
      pendingView = null;
    }
  }

  function discardAndContinue() {
    if (!pendingView) return;
    settingsActions?.discard();
    settingsDirty = false;
    view = pendingView;
    pendingView = null;
  }
  let dictation = $state<StatePayload>({
    stage: "idle",
    recording_started_ms: null,
    message: null,
    error: null,
    last_entry: null,
  });
  let levels = $state<number[]>(Array(LEVEL_BARS).fill(0));
  let historyVersion = $state(0);

  onMount(() => {
    const unsubs: Array<() => void> = [];
    api.getState().then((s) => (dictation = s)).catch(() => {});
    listen<StatePayload>(EVENT_STATE, (event) => {
      dictation = event.payload;
      if (event.payload.stage === "idle") {
        levels = Array(LEVEL_BARS).fill(0);
      }
    }).then((u) => unsubs.push(u));
    listen<number>(EVENT_LEVEL, (event) => {
      levels = [...levels.slice(1), event.payload];
    }).then((u) => unsubs.push(u));
    listen(EVENT_HISTORY, () => {
      historyVersion += 1;
    }).then((u) => unsubs.push(u));
    return () => unsubs.forEach((u) => u());
  });

  const navItems: { id: View; label: string }[] = [
    { id: "dashboard", label: "Dictate" },
    { id: "history", label: "History" },
    { id: "settings", label: "Settings" },
  ];
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <img src="/brand/pickscribe-mark-128.svg" alt="PickScribe" width="34" height="34" />
      <div class="brand-text">
        <span class="brand-name">PickScribe</span>
        <span class="brand-sub">PICKFORGE STUDIO</span>
      </div>
    </div>

    <nav class="nav">
      {#each navItems as item (item.id)}
        <button
          class="nav-item"
          class:active={view === item.id}
          onclick={() => navigate(item.id)}
        >
          {#if item.id === "settings" && settingsDirty}
            <span class="dirty-dot" title="Unsaved changes"></span>
          {/if}
          {#if item.id === "dashboard"}
            <Microphone size={17} weight={view === item.id ? "fill" : "regular"} />
          {:else if item.id === "history"}
            <ClockCounterClockwise size={17} weight={view === item.id ? "fill" : "regular"} />
          {:else}
            <GearSix size={17} weight={view === item.id ? "fill" : "regular"} />
          {/if}
          {item.label}
        </button>
      {/each}
    </nav>

    <footer class="side-foot">
      <span class="pill" class:ember={dictation.stage !== "idle"}>
        {#if dictation.stage !== "idle"}<span class="dot pulse"></span>{/if}
        {dictation.stage}
      </span>
      <p class="local-note">Local-first dictation</p>
    </footer>
  </aside>

  <main class="content">
    {#if view === "dashboard"}
      <Dashboard {dictation} {levels} {historyVersion} />
    {:else if view === "history"}
      <History {historyVersion} />
    {:else}
      <Settings
        onDirtyChange={(dirty) => (settingsDirty = dirty)}
        bindActions={(actions) => (settingsActions = actions)}
      />
    {/if}
  </main>
</div>

{#if pendingView}
  <div class="dialog-backdrop" role="presentation" onclick={() => (pendingView = null)}>
    <div
      class="dialog card"
      role="alertdialog"
      aria-label="Unsaved settings"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => event.key === "Escape" && (pendingView = null)}
    >
      <h3>Unsaved settings</h3>
      <p>You changed settings but haven't saved them yet.</p>
      <div class="dialog-actions">
        <button class="btn btn-ghost btn-sm" onclick={() => (pendingView = null)}>
          Keep editing
        </button>
        <button class="btn btn-danger btn-sm" onclick={discardAndContinue}>Discard</button>
        <button class="btn btn-primary btn-sm" onclick={saveAndContinue}>Save and continue</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 216px minmax(0, 1fr);
    height: 100vh;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding: 22px 14px;
    border-right: 1px solid var(--hairline);
    background: var(--surface-1);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px;
  }

  .brand-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .brand-name {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .brand-sub {
    font-family: var(--font-mono);
    font-size: 8.5px;
    letter-spacing: 0.18em;
    color: var(--muted);
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 38px;
    padding: 0 12px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: color-mix(in srgb, var(--text) 65%, transparent);
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    cursor: pointer;
    transition:
      background 300ms var(--ease-forge),
      color 300ms var(--ease-forge);
  }
  .nav-item:hover {
    color: var(--text);
    background: var(--wash);
  }
  .nav-item.active {
    color: var(--ember);
    background: color-mix(in srgb, var(--ember) 8%, transparent);
  }

  .side-foot {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 0 8px;
  }

  .local-note {
    font-size: 11px;
    color: var(--muted);
  }

  .content {
    overflow-y: auto;
    padding: 28px 32px 40px;
  }

  .dirty-dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--ember);
    flex: none;
    margin-left: -4px;
    animation: ember-pulse 2.4s var(--ease-forge) infinite;
  }

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(4px);
    animation: backdrop-in 250ms var(--ease-forge) both;
  }

  @keyframes backdrop-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .dialog {
    width: min(420px, calc(100vw - 48px));
    padding: 22px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    animation: dialog-in 350ms var(--ease-forge) both;
  }

  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: translateY(14px) scale(0.97);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .dialog h3 {
    font-size: 16px;
  }

  .dialog p {
    font-size: 13px;
    color: var(--muted);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
</style>
