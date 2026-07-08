<script lang="ts">
  import { hostPlatform } from "../platform";
  import WindowControls from "./WindowControls.svelte";

  let { stage, active }: { stage: string; active: boolean } = $props();

  const controlsLeft = hostPlatform() === "macos";
</script>

<header
  class="pf-titlebar"
  class:pf-titlebar--controls-left={controlsLeft}
  class:pf-titlebar--brand-right={controlsLeft}
  data-tauri-drag-region
>
  <div class="pf-titlebar-left" data-tauri-drag-region>
    {#if controlsLeft}
      <WindowControls />
    {:else}
      <div class="pf-brand">
        <span class="pf-mark"></span>
        <span class="pf-wordmark">PickScribe</span>
      </div>
    {/if}
  </div>

  <div data-tauri-drag-region></div>

  <div class="pf-titlebar-right" data-tauri-drag-region>
    <span class="pf-pill">
      <span
        class="pf-dot"
        class:pf-dot--pulsing={active}
        style={`--pf-intent: ${active ? "var(--pf-ember)" : "var(--pf-text-med)"}`}
      ></span>
      {stage}
    </span>
    {#if controlsLeft}
      <div class="pf-brand">
        <span class="pf-mark"></span>
        <span class="pf-wordmark">PickScribe</span>
      </div>
    {:else}
      <WindowControls />
    {/if}
  </div>
</header>
