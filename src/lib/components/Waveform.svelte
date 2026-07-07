<script lang="ts">
  let {
    levels,
    active = false,
    height = 48,
  }: { levels: number[]; active?: boolean; height?: number } = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);

  function channels(varName: string, fallback: string): string {
    const value = getComputedStyle(document.documentElement)
      .getPropertyValue(varName)
      .trim();
    const hex = value || fallback;
    const n = parseInt(hex.slice(1), 16);
    return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
  }

  $effect(() => {
    const el = canvas;
    if (!el) return;
    const parent = el.parentElement;
    const width = parent ? parent.clientWidth : 200;
    const dpr = window.devicePixelRatio || 1;
    el.width = width * dpr;
    el.height = height * dpr;
    el.style.width = `${width}px`;
    el.style.height = `${height}px`;

    const ctx = el.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    const ember = channels("--ember", "#ff7a1a");
    const text = channels("--text", "#f2f2f3");

    const count = levels.length;
    const gap = 2;
    const barWidth = Math.max(2, (width - gap * (count - 1)) / count);
    const mid = height / 2;

    for (let i = 0; i < count; i++) {
      // Newest samples carry full color; the tail fades out.
      const level = Math.min(1, levels[i]);
      const bar = Math.max(2, level * (height - 6));
      const x = i * (barWidth + gap);
      const recency = i / count;
      ctx.fillStyle = active
        ? `rgba(${ember}, ${0.25 + recency * 0.65})`
        : `rgba(${text}, ${0.12 + recency * 0.13})`;
      ctx.beginPath();
      ctx.roundRect(x, mid - bar / 2, barWidth, bar, barWidth / 2);
      ctx.fill();
    }
  });
</script>

<canvas class="waveform-canvas" bind:this={canvas}></canvas>

<style>
  .waveform-canvas {
    display: block;
  }
</style>
