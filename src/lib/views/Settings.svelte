<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
  import {
    api,
    desktopApiAvailable,
    EVENT_CONFIG,
    formatError,
    type AppConfig,
    type DoctorCheck,
  } from "../api";
  import { hostPlatform } from "../platform";
  import { reconcileExternalSettings, shouldApplySaveResponse } from "../settingsMerge";
  import { settingsPlatformDisplayState, settingsSaveDisplayState } from "../settingsDisplay";
  import { setTheme, type ThemeSetting } from "../theme";

  let {
    onDirtyChange = () => {},
    onSavingChange = () => {},
    bindActions = () => {},
  }: {
    onDirtyChange?: (dirty: boolean) => void;
    onSavingChange?: (saving: boolean) => void;
    bindActions?: (actions: { save: () => Promise<boolean>; discard: () => void }) => void;
  } = $props();

  let config = $state<AppConfig | null>(null);
  let savedJson = $state("");
  let models = $state<string[]>([]);
  let cleanupModels = $state<string[]>([]);
  let modelsMessage = $state<string | null>(null);
  let fetchingModels = $state(false);
  let doctor = $state<DoctorCheck[]>([]);
  let autostart = $state(false);
  let autostartSupported = $state(true);
  let status = $state<string | null>(null);
  let error = $state<string | null>(null);
  let externalNotice = $state<string | null>(null);
  let saving = $state(false);
  let receivedConfigEvent = false;
  let configEventRevision = 0;
  let lastOwnSaveJson: string | null = null;

  const dirty = $derived(
    config !== null && savedJson !== "" && JSON.stringify($state.snapshot(config)) !== savedJson
  );
  const saveDisplay = $derived(settingsSaveDisplayState(dirty));
  const platform = hostPlatform();
  const platformDisplay = settingsPlatformDisplayState(platform);

  $effect(() => {
    onDirtyChange(dirty);
  });

  $effect(() => {
    onSavingChange(saving);
  });

  $effect(() => {
    bindActions({ save, discard });
  });

  $effect(() => {
    if (!dirty) externalNotice = null;
  });

  $effect(() => {
    if (!desktopApiAvailable()) return;
    api.listModels().then((m) => (models = m)).catch(() => {});
    refreshDoctor();
    isEnabled()
      .then((enabled) => (autostart = enabled))
      .catch(() => (autostartSupported = false));
  });

  onMount(() => {
    if (!desktopApiAvailable()) return;

    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<AppConfig>(EVENT_CONFIG, (event) => {
      receivedConfigEvent = true;
      configEventRevision += 1;
      applyExternalConfig(event.payload);
    })
      .then((stop) => {
        if (!active) {
          stop();
          return;
        }
        unlisten = stop;
        loadInitialConfig();
      })
      .catch((err) => {
        if (!active) return;
        error = formatError(err);
        loadInitialConfig();
      });

    return () => {
      active = false;
      unlisten?.();
    };
  });

  function loadInitialConfig() {
    void api
      .getAppConfig()
      .then((loaded) => {
        if (!receivedConfigEvent) {
          config = loaded;
          savedJson = JSON.stringify(loaded);
        }
      })
      .catch((err) => (error = formatError(err)));
  }

  function applyExternalConfig(incoming: AppConfig) {
    const incomingJson = JSON.stringify(incoming);
    if (!config || !savedJson) {
      config = incoming;
      savedJson = incomingJson;
      return;
    }

    const resolution = reconcileExternalSettings(
      JSON.parse(savedJson) as AppConfig,
      $state.snapshot(config) as AppConfig,
      incoming
    );
    config = resolution.config;
    savedJson = JSON.stringify(resolution.baseline);
    externalNotice =
      resolution.keptLocalChanges && incomingJson !== lastOwnSaveJson
        ? "Settings changed elsewhere. External updates were refreshed; your edits were kept."
        : null;
    void setTheme(config.general.theme as ThemeSetting);
  }

  function refreshDoctor() {
    api.runDoctor().then((checks) => (doctor = checks)).catch(() => {});
  }

  async function save(): Promise<boolean> {
    if (!config) return false;
    saving = true;
    error = null;
    status = null;
    const submitted = $state.snapshot(config) as AppConfig;
    const submittedJson = JSON.stringify(submitted);
    const eventRevisionAtStart = configEventRevision;
    lastOwnSaveJson = submittedJson;
    try {
      const updated = await api.updateAppConfig(submitted);
      if (shouldApplySaveResponse(eventRevisionAtStart, configEventRevision)) {
        applyExternalConfig(updated);
      }
      status = "Settings saved";
      setTimeout(() => (status = null), 2500);
      refreshDoctor();
      return true;
    } catch (err) {
      lastOwnSaveJson = null;
      error = formatError(err);
      return false;
    } finally {
      saving = false;
    }
  }

  function discard() {
    if (!savedJson) return;
    config = JSON.parse(savedJson) as AppConfig;
    void setTheme(config.general.theme as ThemeSetting);
    error = null;
    externalNotice = null;
  }

  async function toggleAutostart() {
    try {
      if (autostart) {
        await disable();
        autostart = false;
      } else {
        await enable();
        autostart = true;
      }
    } catch (err) {
      error = formatError(err);
    }
  }

  function modelLabel(path: string): string {
    return path.split("/").pop() ?? path;
  }

  async function fetchCleanupModels() {
    if (!config) return;
    fetchingModels = true;
    modelsMessage = null;
    try {
      cleanupModels = await api.listCleanupModels($state.snapshot(config) as AppConfig);
      modelsMessage = `${cleanupModels.length} models available — pick from the list or keep typing`;
    } catch (err) {
      cleanupModels = [];
      modelsMessage = `Couldn't fetch models (${formatError(err)}) — type the model name manually`;
    } finally {
      fetchingModels = false;
    }
  }
</script>

<section class="settings">
  <header class="head fade-up">
    <div>
      <p class="eyebrow ember pf-eyebrow-row"><span class="pf-eyebrow-tick"></span>§ 03 · Settings</p>
      <h2>Tune PickScribe to your voice</h2>
    </div>
    <div class="head-actions">
      {#if status}<span class="pill ok" role="status">{status}</span>{/if}
      <button
        type="button"
        class="btn btn-primary header-save"
        class:header-save-hidden={saveDisplay.headerSaveHidden}
        disabled={saveDisplay.headerSaveDisabled}
        aria-hidden={saveDisplay.headerSaveHidden}
        tabindex={saveDisplay.headerSaveHidden ? -1 : 0}
        onclick={save}
      >
        Save changes
      </button>
    </div>
  </header>

  {#if error}
    <p class="error-line" role="alert">{error}</p>
  {/if}
  {#if externalNotice}
    <p class="external-line" role="status">{externalNotice}</p>
  {/if}

  <div class="panel card">
    <div class="panel-head">
      <h3>System check</h3>
      <button type="button" class="btn btn-ghost btn-sm" onclick={refreshDoctor}>
        <ArrowsClockwise size={13} /> Re-run
      </button>
    </div>
    <div class="doctor-grid">
      {#each doctor as check (check.name)}
        <div class="doctor-item" class:bad={!check.ok}>
          {#if check.ok}
            <CheckCircle size={16} weight="fill" color="var(--ok)" />
          {:else}
            <WarningCircle size={16} weight="fill" color="var(--bad)" />
          {/if}
          <div>
            <p class="doctor-name">{check.name}</p>
            <p class="doctor-detail">{check.detail}</p>
          </div>
        </div>
      {/each}
    </div>
  </div>

  {#if config}
    <div class="panel card">
      <h3>General</h3>
      <div class="rows">
        <div class="row">
          <div>
            <p class="row-label">Theme</p>
            <p class="hint">Dark is the canonical Pickforge mode; System follows your desktop.</p>
          </div>
          <select
            class="select theme-select"
            aria-label="Theme"
            bind:value={config.general.theme}
            onchange={() => setTheme(config!.general.theme as ThemeSetting)}
          >
            <option value="system">System</option>
            <option value="dark">Dark</option>
            <option value="light">Light</option>
          </select>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Start and stop sounds</p>
            <p class="hint">Short ember chimes replace desktop notifications.</p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.general.sounds}
            aria-label="Start and stop sounds"
            onclick={() => (config!.general.sounds = !config!.general.sounds)}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Floating button</p>
            <p class="hint">
              Draggable always-on-top capsule with a live waveform. Click opens the app,
              right-click toggles dictation, middle-click hides it. Also in the tray menu.
            </p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.general.float_button}
            aria-label="Floating button"
            onclick={() => (config!.general.float_button = !config!.general.float_button)}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Launch at login</p>
            <p class="hint">Starts hidden in the tray.</p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={autostart}
            aria-label="Launch at login"
            disabled={!autostartSupported}
            onclick={toggleAutostart}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Typing speed baseline</p>
            <p class="hint">Used to estimate the time you save by dictating.</p>
          </div>
          <div class="inline-input">
            <input
              class="input"
              type="number"
              min="10"
              max="160"
              bind:value={config.general.typing_wpm}
            />
            <span class="unit">WPM</span>
          </div>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Local-only mode</p>
            <p class="hint">
              Nothing ever leaves this machine. Remote cleanup endpoints are blocked — use Ollama,
              a local endpoint, or paste the raw transcript.
            </p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.general.local_only}
            aria-label="Local-only mode"
            onclick={() => (config!.general.local_only = !config!.general.local_only)}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Crash reports</p>
            <p class="hint">
              Send anonymous crash and error reports to help fix problems. Applies after restart.
              {#if config.general.local_only}
                <br />Disabled in local-only mode
              {/if}
            </p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.general.crash_reports && !config.general.local_only}
            aria-label="Crash reports"
            disabled={config.general.local_only}
            onclick={() => (config!.general.crash_reports = !config!.general.crash_reports)}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Keep audio files</p>
            <p class="hint">Recordings are deleted after transcription by default.</p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.general.keep_audio}
            aria-label="Keep audio files"
            onclick={() => (config!.general.keep_audio = !config!.general.keep_audio)}
          ></button>
        </div>
      </div>
      <div class="rows">
        <div class="row">
          <div>
            <p class="row-label">Incremental transcription</p>
            <p class="hint">Show finalized local transcript segments while recording.</p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.incremental.enabled}
            aria-label="Incremental transcription"
            onclick={() => {
              config!.incremental.enabled = !config!.incremental.enabled;
              if (!config!.incremental.enabled) config!.incremental.cleanup_segments = false;
            }}
          ></button>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Clean partial transcript segments</p>
            <p class="hint">
              Experimental. Sends finalized partial text to the cleanup provider before stop and
              falls back to raw text when cleanup drifts. Local-only mode still blocks remote
              endpoints.
            </p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.incremental.cleanup_segments}
            aria-label="Clean partial transcript segments"
            disabled={!config.incremental.enabled}
            onclick={() =>
              (config!.incremental.cleanup_segments = !config!.incremental.cleanup_segments)}
          ></button>
        </div>
      </div>
    </div>

    <div class="panel card">
      <h3>Transcription</h3>
      <div class="grid-2">
        <div class="field">
          <label for="stt-language">Language</label>
          <select id="stt-language" class="select" bind:value={config.stt.language}>
            <option value="auto">Auto-detect</option>
            <option value="en">English</option>
            <option value="pt">Português (BR)</option>
          </select>
        </div>
        <div class="field">
          <label for="stt-model">Whisper model</label>
          <select id="stt-model" class="select" bind:value={config.stt.model_path}>
            <option value="">Auto-detect best model</option>
            {#each models as model (model)}
              <option value={model}>{modelLabel(model)}</option>
            {/each}
          </select>
        </div>
        <div class="field">
          <label for="stt-target">Audio input target</label>
          <input
            id="stt-target"
            class="input"
            type="text"
            placeholder="default microphone"
            bind:value={config.stt.audio_target}
          />
          <span class="hint">
            {platform === "macos"
              ? "AVFoundation audio device index or name; empty uses the system default."
              : "PipeWire node name, leave empty for the default source."}
          </span>
        </div>
      </div>
    </div>

    <div class="panel card">
      <h3>AI cleanup</h3>
      {#if config.general.local_only && (config.cleanup.provider === "deepseek" || config.cleanup.provider === "openai" || config.cleanup.provider === "auto")}
        <p class="local-note-banner">
          Local-only mode is on — remote providers are blocked. Auto resolves to Ollama; DeepSeek
          and OpenAI will fall back to the raw transcript.
        </p>
      {/if}
      <div class="grid-2">
        <div class="field">
          <label for="clean-provider">Provider</label>
          <select id="clean-provider" class="select" bind:value={config.cleanup.provider}>
            <option value="auto">Auto (prefer DeepSeek)</option>
            <option value="deepseek">DeepSeek</option>
            <option value="openai">OpenAI</option>
            <option value="ollama">Ollama (local)</option>
            <option value="custom">Custom endpoint (OpenAI-compatible)</option>
            <option value="none">Off — paste raw transcript</option>
          </select>
          {#if config.cleanup.provider === "custom"}
            <span class="hint">Bring your own API: OpenRouter, OpenCode, LM Studio, vLLM, llama.cpp server…</span>
          {/if}
        </div>
        <div class="field">
          <label for="clean-model">Model</label>
          <div class="model-row">
            <input
              id="clean-model"
              class="input"
              type="text"
              list="cleanup-model-list"
              placeholder={config.cleanup.provider === "ollama"
                ? "qwen2.5:14b"
                : "deepseek-v4-flash"}
              bind:value={config.cleanup.model}
            />
            <button
              type="button"
              class="btn btn-secondary btn-sm"
              onclick={fetchCleanupModels}
              disabled={fetchingModels}
              title="Query the provider's /models route with your key"
            >
              {fetchingModels ? "Fetching…" : "Fetch models"}
            </button>
          </div>
          <datalist id="cleanup-model-list">
            {#each cleanupModels as model (model)}
              <option value={model}></option>
            {/each}
          </datalist>
          {#if modelsMessage}
            <span class="hint">{modelsMessage}</span>
          {/if}
        </div>
        <div class="field">
          <label for="clean-key">API key</label>
          <input
            id="clean-key"
            class="input"
            type="password"
            placeholder="uses DEEPSEEK_API_KEY / ~/.config/pickscribe/env"
            bind:value={config.cleanup.api_key}
          />
          <span class="hint">Environment variables and the env file take precedence.</span>
        </div>
        <div class="field">
          <label for="clean-endpoint">
            {config.cleanup.provider === "custom" ? "Endpoint (required)" : "Custom endpoint"}
          </label>
          <input
            id="clean-endpoint"
            class="input"
            type="text"
            placeholder={config.cleanup.provider === "custom"
              ? "http://127.0.0.1:11434/v1/chat/completions"
              : "provider default"}
            bind:value={config.cleanup.endpoint}
          />
          <span class="hint">Full chat completions URL; models are discovered from the matching /models route.</span>
        </div>
        <div class="field">
          <label for="clean-timeout">Timeout</label>
          <div class="inline-input">
            <input
              id="clean-timeout"
              class="input"
              type="number"
              min="5"
              max="120"
              bind:value={config.cleanup.timeout_secs}
            />
            <span class="unit">seconds</span>
          </div>
          <span class="hint">On failure the raw transcript is pasted instead.</span>
        </div>
        <div class="field">
          <label for="clean-thinking">DeepSeek thinking</label>
          <select id="clean-thinking" class="select" bind:value={config.cleanup.thinking}>
            <option value="disabled">Disabled (fastest)</option>
            <option value="auto">Provider default</option>
            <option value="enabled">Enabled</option>
          </select>
        </div>
      </div>
      <div class="field">
        <label for="clean-instructions">Cleanup instructions</label>
        <textarea
          id="clean-instructions"
          class="input"
          placeholder="Leave empty for the built-in instructions (fix punctuation, grammar, casing; keep the original language)."
          bind:value={config.cleanup.instructions}
        ></textarea>
      </div>
    </div>

    <div class="panel card">
      <h3>Paste</h3>
      <div class="grid-2">
        <div class="field">
          <label for="paste-method">Method</label>
          <select id="paste-method" class="select" bind:value={config.paste.method}>
            <option value="auto">Auto</option>
            <option value="hotkey">Paste hotkey</option>
            <option value="type">Type character by character</option>
            <option value="none">Clipboard only</option>
          </select>
        </div>
        <div class="field">
          <label for="paste-chord">Paste chord</label>
          <select id="paste-chord" class="select" bind:value={config.paste.chord}>
            <option value="ctrl-v">{platform === "macos" ? "Cmd+V" : "Ctrl+V"}</option>
            <option value="ctrl-shift-v">
              {platform === "macos" ? "Cmd+Shift+V" : "Ctrl+Shift+V (terminals)"}
            </option>
          </select>
        </div>
        <div class="field">
          <label for="paste-delay">Delay before paste</label>
          <div class="inline-input">
            <input
              id="paste-delay"
              class="input"
              type="number"
              min="0"
              max="2000"
              step="50"
              bind:value={config.paste.delay_ms}
            />
            <span class="unit">ms</span>
          </div>
          <span class="hint">Gives you time to release the hotkey modifiers.</span>
        </div>
        <div class="row">
          <div>
            <p class="row-label">Always copy to clipboard</p>
            <p class="hint">Keeps the text available even if pasting fails.</p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={config.paste.copy_to_clipboard}
            aria-label="Always copy to clipboard"
            onclick={() => (config!.paste.copy_to_clipboard = !config!.paste.copy_to_clipboard)}
          ></button>
        </div>
      </div>
    </div>

    <!--
      Dirty-state save/discard actions render at the app overlay layer
      (App.svelte), not here. `.content.fade-up` (the scrolling ancestor in
      App.svelte) carries a transform via its entrance animation, which
      makes it a containing block for `position: fixed` descendants — a
      fixed element placed inside it would be positioned/clipped relative to
      that scroller instead of the app viewport. See issue #45.
    -->

    {#if platformDisplay.shortcutFieldVisible}
      <div class="panel card hotkey-panel">
        <h3>Global shortcut</h3>
        <div class="field">
          <label for="shortcut-toggle">Toggle dictation</label>
          <input
            id="shortcut-toggle"
            class="input"
            type="text"
            placeholder="Cmd+Shift+Space"
            bind:value={config.shortcut.toggle}
          />
          <span class="hint">Use a shortcut such as Cmd+Shift+Space. Leave empty to turn it off.</span>
        </div>
      </div>
    {:else if platformDisplay.desktopKeybindingHelpVisible}
      <div class="panel card hotkey-panel">
        <h3>Global hotkey</h3>
        <p class="hint">
          Bind a key in KDE System Settings → Shortcuts → Custom Shortcuts to:
        </p>
        <code class="hotkey-code">pickscribe-app --toggle</code>
        <p class="hint">
          Press once to start recording, again to stop. A remapped Caps Lock (to F13) makes a great
          dedicated dictation key. The CLI <code>pickscribe</code> binary keeps working too.
        </p>
      </div>
    {/if}
  {/if}
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
  }

  .head h2 {
    font-size: 24px;
    margin-top: 4px;
  }

  .head-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .error-line,
  .external-line {
    font-size: 13px;
  }

  .error-line {
    color: var(--bad);
  }

  .external-line {
    color: var(--muted);
  }

  .panel {
    padding: 22px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .panel h3 {
    font-size: 16px;
  }

  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .doctor-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 10px;
  }

  .doctor-item {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--wash);
    border: 1px solid var(--hairline);
  }
  .doctor-item.bad {
    border-color: rgba(251, 113, 133, 0.3);
    background: rgba(251, 113, 133, 0.04);
  }

  .doctor-name {
    font-size: 13px;
    font-weight: 600;
  }

  .doctor-detail {
    font-size: 11.5px;
    color: var(--muted);
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
  }

  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 12px 0;
    border-top: 1px solid var(--hairline);
  }
  .rows .row:first-child {
    border-top: none;
    padding-top: 0;
  }

  .row-label {
    font-size: 13.5px;
    font-weight: 600;
  }

  .hint {
    font-size: 12px;
    color: var(--muted);
    line-height: 1.5;
  }

  .grid-2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 16px;
  }

  @media (max-width: 860px) {
    .grid-2 {
      grid-template-columns: 1fr;
    }
  }

  .inline-input {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .inline-input .input {
    width: 110px;
  }

  .unit {
    font-family: var(--font-mono);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--muted);
  }

  .switch:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .theme-select {
    width: 140px;
  }

  .header-save-hidden {
    visibility: hidden;
  }

  .model-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .model-row .input {
    flex: 1;
    min-width: 0;
  }

  .model-row .btn {
    flex: none;
  }

  .local-note-banner {
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--ember);
    background: color-mix(in srgb, var(--ember) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--ember) 30%, transparent);
    border-radius: 10px;
    padding: 10px 14px;
  }

  .hotkey-panel {
    gap: 10px;
  }

  .hotkey-code {
    align-self: flex-start;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--ember);
    background: color-mix(in srgb, var(--ember) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--ember) 30%, transparent);
    border-radius: 8px;
    padding: 8px 14px;
  }
</style>
