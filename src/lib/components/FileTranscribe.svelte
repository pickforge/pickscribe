<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import FileArrowDown from "phosphor-svelte/lib/FileArrowDown";
  import FileAudio from "phosphor-svelte/lib/FileAudio";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import X from "phosphor-svelte/lib/X";
  import {
    api,
    desktopApiAvailable,
    formatError,
    onFileJob,
    type FileJobState,
  } from "../api";
  import {
    basename,
    fileStageLabel,
    isDeterminate,
    isMediaPath,
    pickMediaPaths,
  } from "../file-transcribe";

  let {
    bindActions,
    onViewHistory,
    onBusyChange,
  }: {
    bindActions: (actions: { browse: () => void }) => void;
    onViewHistory: () => void;
    onBusyChange?: (busy: boolean) => void;
  } = $props();

  let pendingFile = $state<string | null>(null);
  let cleanup = $state(false);
  let job = $state<FileJobState | null>(null);
  let startError = $state<string | null>(null);
  let dragActive = $state(false);
  let dialogEl = $state<HTMLDivElement | null>(null);

  const displayName = $derived(basename(pendingFile ?? job?.source_file ?? ""));
  const displayPath = $derived(pendingFile ?? job?.source_file ?? "");
  const visible = $derived(pendingFile !== null || job !== null);
  const running = $derived(
    job !== null &&
      (job.stage === "converting" || job.stage === "transcribing" || job.stage === "cleaning")
  );

  function isBusy() {
    return pendingFile !== null || job !== null;
  }

  function openConfirm(path: string) {
    job = null;
    startError = null;
    cleanup = false;
    pendingFile = path;
  }

  function dismiss() {
    pendingFile = null;
    job = null;
    startError = null;
    cleanup = false;
  }

  function requestDismiss() {
    if (running) return;
    dismiss();
  }

  async function browse() {
    if (isBusy()) return;
    try {
      const path = await api.pickMediaFile();
      if (path && isMediaPath(path)) {
        openConfirm(path);
      }
    } catch {
      // Native dialog cancellation surfaces as a rejection on some platforms.
    }
  }

  async function start() {
    if (!pendingFile) return;
    const path = pendingFile;
    const withCleanup = cleanup;
    startError = null;
    job = { stage: "converting", progress: 0, source_file: path, error: null, entry_id: null };
    try {
      await api.transcribeMediaFile(path, withCleanup);
    } catch (err) {
      job = null;
      startError = formatError(err);
    }
  }

  function cancelJob() {
    api.cancelFileTranscription().catch(() => {});
  }

  function viewHistory() {
    onViewHistory();
    dismiss();
  }

  $effect(() => {
    onBusyChange?.(visible);
  });

  $effect(() => {
    bindActions({ browse });
  });

  $effect(() => {
    if (visible) {
      dialogEl?.focus();
    }
  });

  onMount(() => {
    if (!desktopApiAvailable()) return;
    const unsubs: Array<() => void> = [];

    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter") {
          if (!isBusy() && pickMediaPaths(payload.paths).length > 0) {
            dragActive = true;
          }
        } else if (payload.type === "leave") {
          dragActive = false;
        } else if (payload.type === "drop") {
          dragActive = false;
          if (isBusy()) return;
          const media = pickMediaPaths(payload.paths);
          if (media.length > 0) {
            openConfirm(media[0]);
          }
        }
      })
      .then((u) => unsubs.push(u));

    onFileJob((state) => {
      job = state;
    }).then((u) => unsubs.push(u));

    return () => unsubs.forEach((u) => u());
  });
</script>

{#if dragActive}
  <div class="drop-overlay" role="presentation">
    <div class="drop-frame">
      <FileArrowDown size={40} weight="regular" />
      <p class="drop-title">Drop to transcribe</p>
      <p class="drop-hint">Audio or video · stays on this machine</p>
    </div>
  </div>
{/if}

{#if visible}
  <div class="dialog-backdrop" role="presentation" onclick={requestDismiss}>
    <div
      class="dialog card"
      role="dialog"
      aria-modal="true"
      aria-label="File transcription"
      tabindex="-1"
      bind:this={dialogEl}
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => event.key === "Escape" && requestDismiss()}
    >
      {#if job}
        {#if job.stage === "done"}
          <div class="status-icon ok"><CheckCircle size={26} weight="fill" /></div>
          <h3>Transcription saved</h3>
          <p class="dialog-sub" title={displayPath}>{displayName}</p>
          {#if job.error}
            <p class="dialog-note" title={job.error}>{job.error}</p>
          {/if}
          <div class="dialog-actions">
            <button class="btn btn-ghost btn-sm" type="button" onclick={dismiss}>Dismiss</button>
            <button class="btn btn-primary btn-sm" type="button" onclick={viewHistory}>
              View in History
            </button>
          </div>
        {:else if job.stage === "error"}
          <div class="status-icon bad"><WarningCircle size={26} weight="fill" /></div>
          <h3>Transcription failed</h3>
          <p class="dialog-error">{job.error ?? "Something went wrong."}</p>
          <div class="dialog-actions">
            <button class="btn btn-ghost btn-sm" type="button" onclick={dismiss}>Dismiss</button>
          </div>
        {:else if job.stage === "cancelled"}
          <h3>Transcription cancelled</h3>
          <p class="dialog-sub" title={displayPath}>{displayName}</p>
          <div class="dialog-actions">
            <button class="btn btn-ghost btn-sm" type="button" onclick={dismiss}>Dismiss</button>
          </div>
        {:else}
          <p class="eyebrow ember">File transcription</p>
          <div class="running-head">
            <FileAudio size={18} weight="regular" />
            <span class="running-name" title={displayPath}>{displayName}</span>
          </div>
          <div class="progress" class:indeterminate={!isDeterminate(job)}>
            <div
              class="progress-fill"
              style={isDeterminate(job) ? `width: ${job.progress}%` : undefined}
            ></div>
          </div>
          <div class="dialog-actions between">
            <span class="stage-label">{fileStageLabel(job)}</span>
            <button class="btn btn-ghost btn-sm" type="button" onclick={cancelJob}>
              <X size={13} /> Cancel
            </button>
          </div>
        {/if}
      {:else if pendingFile}
        <p class="eyebrow ember">Transcribe a file</p>
        <div class="confirm-file">
          <FileAudio size={20} weight="regular" />
          <span class="confirm-name" title={displayPath}>{displayName}</span>
        </div>
        <div class="cleanup-row">
          <div class="cleanup-copy">
            <p class="cleanup-label"><Sparkle size={14} weight="fill" /> Clean up with AI</p>
            <p class="cleanup-hint">
              Off keeps it fully local — no text is sent to a cleanup model.
            </p>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={cleanup}
            aria-label="Clean up with AI"
            onclick={() => (cleanup = !cleanup)}
          ></button>
        </div>
        {#if startError}
          <p class="dialog-error">{startError}</p>
        {/if}
        <div class="dialog-actions">
          <button class="btn btn-ghost btn-sm" type="button" onclick={dismiss}>Cancel</button>
          <button class="btn btn-primary btn-sm" type="button" onclick={start}>Transcribe</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .drop-overlay {
    position: fixed;
    inset: 0;
    z-index: 120;
    display: grid;
    place-items: center;
    padding: 20px;
    background: color-mix(in srgb, var(--surface) 72%, transparent);
    backdrop-filter: blur(3px);
    animation: backdrop-in 180ms var(--ease-forge) both;
    pointer-events: none;
  }

  .drop-frame {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: min(440px, calc(100vw - 64px));
    padding: 40px 32px;
    border-radius: var(--radius-card);
    border: 1.5px dashed color-mix(in srgb, var(--ember) 55%, transparent);
    background: color-mix(in srgb, var(--ember) 7%, transparent);
    color: var(--ember);
  }

  .drop-title {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text);
  }

  .drop-hint {
    font-size: 12px;
    color: var(--muted);
  }

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 110;
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
    gap: 10px;
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

  .dialog-sub {
    font-size: 13px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog-note {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog-error {
    font-size: 13px;
    line-height: 1.5;
    color: var(--bad);
    overflow-wrap: anywhere;
  }

  .status-icon {
    display: flex;
  }
  .status-icon.ok {
    color: var(--ok);
  }
  .status-icon.bad {
    color: var(--bad);
  }

  .confirm-file {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--well);
    border: 1px solid var(--well-border);
    color: var(--ember);
  }

  .confirm-name,
  .running-name {
    min-width: 0;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cleanup-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 2px 2px 4px;
  }

  .cleanup-copy {
    min-width: 0;
  }

  .cleanup-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
    color: color-mix(in srgb, var(--text) 90%, transparent);
  }
  .cleanup-label :global(svg) {
    color: var(--ember);
  }

  .cleanup-hint {
    margin-top: 3px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--muted);
  }

  .running-head {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ember);
  }

  .progress {
    height: 8px;
    border-radius: 999px;
    background: var(--wash-strong);
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    border-radius: 999px;
    background: linear-gradient(90deg, var(--ember-soft), var(--ember-deep));
    transition: width 300ms var(--ease-forge);
  }

  .progress.indeterminate .progress-fill {
    width: 40%;
    animation: progress-slide 1.2s var(--ease-forge) infinite;
  }

  @keyframes progress-slide {
    0% {
      transform: translateX(-110%);
    }
    100% {
      transform: translateX(260%);
    }
  }

  .stage-label {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--muted);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .dialog-actions.between {
    justify-content: space-between;
    align-items: center;
  }
</style>
