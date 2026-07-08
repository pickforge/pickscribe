<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { hostPlatform } from "../platform";

  const MAXIMIZED_CHECK_DELAY_MS = 120;

  const isWeb = hostPlatform() === "web";
  const isMac = hostPlatform() === "macos";

  let maximized = $state(false);

  onMount(() => {
    if (isWeb) {
      return;
    }
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    let checking = false;

    const readMaximized = async () => {
      if (disposed || checking) {
        return;
      }
      checking = true;
      try {
        maximized = await win.isMaximized();
      } catch {
        /* window closing */
      } finally {
        checking = false;
      }
    };
    const scheduleRead = () => {
      if (timer) {
        clearTimeout(timer);
      }
      timer = setTimeout(() => {
        timer = undefined;
        void readMaximized();
      }, MAXIMIZED_CHECK_DELAY_MS);
    };

    void readMaximized();
    void win.onResized(scheduleRead).then((off) => {
      if (disposed) {
        off();
      } else {
        unlisten = off;
      }
    });

    return () => {
      disposed = true;
      if (timer) {
        clearTimeout(timer);
      }
      unlisten?.();
    };
  });

  const minimize = () =>
    void getCurrentWindow()
      .minimize()
      .catch(() => {});
  const toggleMax = () =>
    void getCurrentWindow()
      .toggleMaximize()
      .then(async () => {
        maximized = await getCurrentWindow().isMaximized();
      })
      .catch(() => {});
  const close = () =>
    void getCurrentWindow()
      .close()
      .catch(() => {});
</script>

{#snippet icon(kind: "minimize" | "maximize" | "restore" | "close")}
  <svg
    class="pf-winctl-icon"
    width="10"
    height="10"
    viewBox="0 0 10 10"
    fill="none"
    stroke="currentColor"
    stroke-width="1.1"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {#if kind === "minimize"}<path d="M1.5 5h7" />{/if}
    {#if kind === "maximize"}<rect x="1.5" y="1.5" width="7" height="7" rx="0.6" />{/if}
    {#if kind === "restore"}<path d="M3 3V1.6h5.4V7H7" /><rect x="1.5" y="3" width="5.5" height="5.5" rx="0.6" />{/if}
    {#if kind === "close"}<path d="M1.8 1.8l6.4 6.4M8.2 1.8l-6.4 6.4" />{/if}
  </svg>
{/snippet}

{#snippet minimizeBtn()}
  <button type="button" class="pf-winctl-btn" title="Minimize" aria-label="Minimize" onclick={minimize}>
    {@render icon("minimize")}
  </button>
{/snippet}
{#snippet maximizeBtn()}
  <button
    type="button"
    class="pf-winctl-btn"
    title={maximized ? "Restore" : "Maximize"}
    aria-label={maximized ? "Restore" : "Maximize"}
    onclick={toggleMax}
  >
    {@render icon(maximized ? "restore" : "maximize")}
  </button>
{/snippet}
{#snippet closeBtn()}
  <button type="button" class="pf-winctl-btn pf-winctl-btn--close" title="Close" aria-label="Close" onclick={close}>
    {@render icon("close")}
  </button>
{/snippet}

{#if !isWeb}
  <div class="pf-winctl" role="group" aria-label="Window controls">
    {#if isMac}
      {@render closeBtn()}
      {@render minimizeBtn()}
      {@render maximizeBtn()}
    {:else}
      {@render minimizeBtn()}
      {@render maximizeBtn()}
      {@render closeBtn()}
    {/if}
  </div>
{/if}
