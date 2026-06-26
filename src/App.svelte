<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import GearSix from "phosphor-svelte/lib/GearSix";
  import Microphone from "phosphor-svelte/lib/Microphone";
  import {
    api,
    desktopApiAvailable,
    EVENT_HISTORY,
    EVENT_LEVEL,
    EVENT_STATE,
    type StatePayload,
  } from "./lib/api";
  import Dashboard from "./lib/views/Dashboard.svelte";
  import History from "./lib/views/History.svelte";
  import Settings from "./lib/views/Settings.svelte";
  import { checkForUpdates } from "./lib/updater";

  type View = "dashboard" | "history" | "settings";

  const LEVEL_BARS = 56;

  let view = $state<View>("dashboard");
  let settingsDirty = $state(false);
  let pendingView = $state<View | null>(null);
  let settingsActions: { save: () => Promise<boolean>; discard: () => void } | null = null;
  let dictation = $state<StatePayload>({
    stage: "idle",
    recording_started_ms: null,
    message: null,
    error: null,
    last_entry: null,
  });
  let levels = $state<number[]>(Array(LEVEL_BARS).fill(0));
  let historyVersion = $state(0);

  const navItems: { id: View; label: string; icon: typeof Microphone }[] = [
    { id: "dashboard", label: "Dictate", icon: Microphone },
    { id: "history", label: "History", icon: ClockCounterClockwise },
    { id: "settings", label: "Settings", icon: GearSix },
  ];

  const active = $derived(dictation.stage !== "idle");

  function navigate(target: View) {
    if (view === "settings" && settingsDirty && target !== "settings") {
      pendingView = target;
      return;
    }
    view = target;
  }

  async function saveAndContinue() {
    if (!settingsActions || !pendingView) {
      return;
    }
    if (await settingsActions.save()) {
      view = pendingView;
    }
    pendingView = null;
  }

  function discardAndContinue() {
    if (!pendingView) {
      return;
    }
    settingsActions?.discard();
    settingsDirty = false;
    view = pendingView;
    pendingView = null;
  }

  onMount(() => {
    if (!desktopApiAvailable()) {
      return;
    }

    const unsubs: Array<() => void> = [];

    // Autostart's "Launch at login" starts the app `--hidden`, which hides the
    // main window — so a blocking update confirm() must not pop from an
    // invisible webview. Only this main window runs the check (the float capsule
    // mounts Float, not App), and only while visible; otherwise defer it until
    // the window is first shown.
    let updateCheckDone = false;
    const runUpdateCheck = () => {
      if (updateCheckDone) {
        return;
      }
      updateCheckDone = true;
      void checkForUpdates();
    };
    const mainWindow = getCurrentWindow();
    void mainWindow.isVisible().then((visible) => {
      if (visible) {
        runUpdateCheck();
        return;
      }
      mainWindow
        .onFocusChanged(({ payload: focused }) => {
          if (focused) {
            runUpdateCheck();
          }
        })
        .then((u) => unsubs.push(u));
    });

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
</script>

<div class="app bg-blueprint">
  <header class="chrome">
    <div class="chrome-dots" aria-hidden="true">
      <span></span>
      <span></span>
      <span></span>
    </div>
    <span class="chrome-title">Pickscribe · Dictation</span>
    <span class="pill" class:ember={active}>
      <span class="dot" class:pulse={active}></span>
      {dictation.stage}
    </span>
  </header>

  <div class="body">
    <aside class="sidebar">
      <img class="mark" src="/brand/pickscribe-mark-128.svg" alt="PickScribe mark" />
      <nav aria-label="Main navigation">
        {#each navItems as item (item.id)}
          <button
            class="nav-btn"
            class:active={view === item.id}
            type="button"
            onclick={() => navigate(item.id)}
          >
            {#if item.id === "settings" && settingsDirty}
              <span class="dirty-dot" title="Unsaved changes"></span>
            {/if}
            <item.icon size={17} weight={view === item.id ? "fill" : "regular"} />
            {item.label}
          </button>
        {/each}
      </nav>
    </aside>

    <main class="content fade-up">
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

  <footer class="footer">
    <span class="status" class:error={Boolean(dictation.error)}>
      {dictation.error ?? dictation.message ?? "Local-first dictation"}
    </span>
    <span class="brand-line">© Pickforge · pickforge.dev · MIT</span>
  </footer>
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
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => (pendingView = null)}>
          Keep editing
        </button>
        <button class="btn btn-danger btn-sm" type="button" onclick={discardAndContinue}>
          Discard
        </button>
        <button class="btn btn-primary btn-sm" type="button" onclick={saveAndContinue}>
          Save and continue
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background-color: var(--surface);
  }

  .chrome {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: none;
    height: 44px;
    padding: 0 16px;
    border-bottom: 1px solid var(--hairline);
    background: color-mix(in srgb, var(--surface) 75%, transparent);
  }

  .chrome-dots {
    display: flex;
    gap: 6px;
  }

  .chrome-dots span {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--text) 15%, transparent);
  }

  .chrome-title {
    flex: 1;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 20px;
    flex: none;
    width: 176px;
    padding: 18px 12px;
    border-right: 1px solid var(--hairline);
    background: color-mix(in srgb, var(--surface-1) 55%, transparent);
  }

  .mark {
    width: 34px;
    height: 34px;
    margin-left: 6px;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 12px;
    border: none;
    border-radius: 9px;
    background: transparent;
    color: var(--muted);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    cursor: pointer;
    transition:
      background 0.3s var(--ease-forge),
      color 0.3s var(--ease-forge);
  }

  .nav-btn:hover {
    color: var(--text);
    background: var(--wash);
  }

  .nav-btn.active {
    color: var(--ember);
    background: color-mix(in srgb, var(--ember) 8%, transparent);
  }

  .nav-btn:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--ember) 60%, transparent);
    outline-offset: -2px;
  }

  .dirty-dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-pill);
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

  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 24px 28px 32px;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex: none;
    height: 34px;
    padding: 0 16px;
    border-top: 1px solid var(--hairline);
    background: color-mix(in srgb, var(--surface) 75%, transparent);
  }

  .status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--muted);
  }

  .status.error {
    color: var(--bad);
  }

  .brand-line {
    flex: none;
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--muted);
  }

  @media (max-width: 700px) {
    .sidebar {
      width: 60px;
      padding: 18px 8px;
      align-items: center;
    }

    .mark {
      margin-left: 0;
    }

    .nav-btn {
      justify-content: center;
      width: 44px;
      padding: 0;
      font-size: 0;
      gap: 0;
    }

    .content {
      padding: 18px 14px 24px;
    }
  }
</style>
