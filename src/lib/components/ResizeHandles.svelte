<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { desktopApiAvailable } from "../api";

  const HANDLES = [
    { dir: "North", cls: "n" },
    { dir: "South", cls: "s" },
    { dir: "East", cls: "e" },
    { dir: "West", cls: "w" },
    { dir: "NorthWest", cls: "nw" },
    { dir: "NorthEast", cls: "ne" },
    { dir: "SouthWest", cls: "sw" },
    { dir: "SouthEast", cls: "se" },
  ] as const;

  const start = (dir: string) => async (event: MouseEvent) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    try {
      await getCurrentWindow().startResizeDragging(dir as never);
    } catch {
      /* not in Tauri / permission denied */
    }
  };
</script>

{#if desktopApiAvailable()}
  {#each HANDLES as handle (handle.cls)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class={`pf-resize pf-resize--${handle.cls}`} onmousedown={start(handle.dir)}></div>
  {/each}
{/if}
