<script lang="ts">
  import Microphone from "phosphor-svelte/lib/Microphone";
  import Stop from "phosphor-svelte/lib/Stop";
  import X from "phosphor-svelte/lib/X";
  import {
    api,
    desktopApiAvailable,
    formatDuration,
    formatMinutes,
    formatError,
    segmentDisplayText,
    segmentStatusLabel,
    type Metrics,
    type PlatformSupport,
    type StatePayload,
  } from "../api";
  import Waveform from "../components/Waveform.svelte";

  let {
    dictation,
    levels,
    historyVersion,
  }: { dictation: StatePayload; levels: number[]; historyVersion: number } = $props();

  let metrics = $state<Metrics | null>(null);
  let metricsError = $state<string | null>(null);
  let localOnly = $state(false);
  let platformSupport = $state<PlatformSupport | null>(null);
  let nowMs = $state(Date.now());

  $effect(() => {
    if (!desktopApiAvailable()) return;
    api.getAppConfig().then((c) => (localOnly = c.general.local_only)).catch(() => {});
    api.getPlatformSupport().then((support) => (platformSupport = support)).catch(() => {});
  });

  $effect(() => {
    void historyVersion;
    if (!desktopApiAvailable()) return;
    api
      .getMetrics()
      .then((m) => {
        metrics = m;
        metricsError = null;
      })
      .catch((err) => (metricsError = formatError(err)));
  });

  $effect(() => {
    if (dictation.stage !== "recording") return;
    const timer = setInterval(() => (nowMs = Date.now()), 250);
    return () => clearInterval(timer);
  });

  const elapsed = $derived(
    dictation.stage === "recording" && dictation.recording_started_ms
      ? formatDuration(Math.max(0, nowMs - dictation.recording_started_ms))
      : null
  );

  const busy = $derived(
    dictation.stage === "transcribing" ||
      dictation.stage === "cleaning" ||
      dictation.stage === "pasting"
  );

  const platformBlocked = $derived(
    platformSupport !== null && !platformSupport.dictation_supported
  );

  const stageLabel = $derived(
    {
      idle: "Ready",
      recording: "Recording",
      transcribing: "Transcribing",
      cleaning: "Cleaning",
      pasting: "Pasting",
    }[dictation.stage]
  );

  const chartDays = $derived.by(() => {
    const map = new Map((metrics?.days ?? []).map((d) => [d.day, d]));
    const days: { day: string; label: string; words: number }[] = [];
    for (let i = 13; i >= 0; i--) {
      const date = new Date();
      date.setDate(date.getDate() - i);
      const key = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
      days.push({
        day: key,
        label: date.toLocaleDateString(undefined, { weekday: "narrow" }),
        words: map.get(key)?.words ?? 0,
      });
    }
    return days;
  });

  const chartMax = $derived(Math.max(1, ...chartDays.map((d) => d.words)));
  const segments = $derived(dictation.segments ?? []);

  function toggle() {
    if (platformBlocked) return;
    api.toggleDictation().catch(() => {});
  }

  function cancel() {
    api.cancelDictation().catch(() => {});
  }
</script>

<section class="dashboard">
  <header class="head fade-up">
    <div>
      <p class="eyebrow ember">§ 01 · Dictation</p>
      <h2>
        {#if platformBlocked}Linux release target only{:else if dictation.stage === "idle"}Press the orb or your hotkey{:else}{stageLabel}…{/if}
      </h2>
    </div>
    <div class="head-pills">
      {#if localOnly}
        <span class="pill ok">LOCAL-ONLY</span>
      {/if}
      <span class="pill" class:ember={dictation.stage !== "idle"}>
        {#if dictation.stage !== "idle"}<span class="dot pulse"></span>{/if}
        {stageLabel}
      </span>
    </div>
  </header>

  <div class="stage card">
    <button
      type="button"
      class="orb"
      class:recording={dictation.stage === "recording"}
      class:busy
      onclick={toggle}
      disabled={busy || platformBlocked}
      aria-label={platformBlocked ? "Dictation is unavailable on this platform" : dictation.stage === "recording" ? "Stop recording" : "Start recording"}
    >
      {#if dictation.stage === "recording"}
        <Stop size={36} weight="fill" />
      {:else if busy}
        <span class="spinner"></span>
      {:else}
        <Microphone size={36} weight="fill" />
      {/if}
    </button>

    <div class="stage-side">
      <div class="wave" class:dim={dictation.stage !== "recording"}>
        <Waveform {levels} active={dictation.stage === "recording"} height={64} />
      </div>
      <div class="stage-meta">
        {#if elapsed}
          <span class="elapsed">{elapsed}</span>
          <button type="button" class="btn btn-ghost btn-sm" onclick={cancel}>
            <X size={13} /> Cancel
          </button>
        {:else if busy}
          <span class="muted-line">{stageLabel} your dictation…</span>
        {:else if dictation.error}
          <span class="error-line" title={dictation.error}>{dictation.error}</span>
        {:else if dictation.message}
          <span class="muted-line">{dictation.message}</span>
        {:else if platformBlocked && platformSupport}
          <span class="error-line" title={platformSupport.summary}>{platformSupport.summary}</span>
        {:else if localOnly}
          <span class="muted-line">Local-only mode — nothing leaves this machine.</span>
        {:else}
          <span class="muted-line">Audio stays on this machine — only text goes to cleanup.</span>
        {/if}
      </div>
    </div>
  </div>

  {#if segments.length > 0}
    <section class="segments card fade-up">
      <div class="segment-head">
        <p class="eyebrow">LIVE TRANSCRIPT</p>
        <span class="pill">{segments.length} segments</span>
      </div>
      <div class="segment-list">
        {#each segments as segment (segment.id)}
          {@const text = segmentDisplayText(segment)}
          <div class="segment-row" class:failed={segment.status === "failed"}>
            <span
              class="segment-status"
              class:pending={segment.status === "transcribing" ||
                segment.status === "provisional" ||
                segment.status === "recording"}
            >
              {segmentStatusLabel(segment.status)}
            </span>
            <p class="segment-text" aria-live="polite">
              {#if segment.status === "failed"}
                {segment.error ?? "Segment failed"}
              {:else if text}
                {text}
              {:else}
                Listening...
              {/if}
            </p>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if metricsError}
    <p class="error-line">{metricsError}</p>
  {:else if metrics}
    <div class="stats">
      <div class="stat card">
        <p class="eyebrow">TIME SAVED</p>
        <p class="stat-value ember-text">{formatMinutes(metrics.minutes_saved)}</p>
        <p class="stat-sub">vs typing at {metrics.typing_wpm} WPM</p>
      </div>
      <div class="stat card">
        <p class="eyebrow">WORDS</p>
        <p class="stat-value">{metrics.words.toLocaleString()}</p>
        <p class="stat-sub">{Math.round(metrics.avg_words_per_session)} avg per session</p>
      </div>
      <div class="stat card">
        <p class="eyebrow">SPOKEN</p>
        <p class="stat-value">{formatMinutes(metrics.speaking_ms / 60000)}</p>
        <p class="stat-sub">longest {formatDuration(metrics.longest_session_ms)}</p>
      </div>
      <div class="stat card">
        <p class="eyebrow">SESSIONS</p>
        <p class="stat-value">{metrics.sessions.toLocaleString()}</p>
        <p class="stat-sub">all time</p>
      </div>
    </div>

    <div class="chart card">
      <div class="chart-head">
        <p class="eyebrow">LAST 14 DAYS</p>
        <span class="chart-total">{chartDays.reduce((acc, d) => acc + d.words, 0).toLocaleString()} words</span>
      </div>
      <div class="bars">
        {#each chartDays as day (day.day)}
          <div class="bar-col" title={`${day.day}: ${day.words} words`}>
            <div class="bar-track">
              <div
                class="bar"
                class:empty={day.words === 0}
                style={`height: ${Math.max(4, (day.words / chartMax) * 100)}%`}
              ></div>
            </div>
            <span class="bar-label">{day.label}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  {#if dictation.last_entry}
    {@const entry = dictation.last_entry}
    <div class="last card fade-up">
      <div class="last-head">
        <p class="eyebrow">LAST TRANSCRIPTION</p>
        <span class="pill">{entry.word_count} words · {formatDuration(entry.duration_ms)}</span>
      </div>
      <div class="last-grid">
        <div class="transcript raw">
          <p class="transcript-label">RAW · WHISPER</p>
          <p class="transcript-text">{entry.raw_text}</p>
        </div>
        {#if entry.cleaned_text}
          <div class="transcript clean">
            <p class="transcript-label ember-text">
              CLEANED · {(entry.provider || "LLM").toUpperCase()}
            </p>
            <p class="transcript-text">{entry.cleaned_text}</p>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .dashboard {
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

  .head-pills {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }

  .stage {
    display: flex;
    align-items: center;
    gap: 28px;
    padding: 28px;
  }

  .orb {
    flex: none;
    width: 116px;
    height: 116px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--ember) 40%, transparent);
    background: radial-gradient(circle at 35% 30%, color-mix(in srgb, var(--ember) 22%, transparent), color-mix(in srgb, var(--ember) 6%, transparent) 70%);
    color: var(--ember);
    display: grid;
    place-items: center;
    cursor: pointer;
    box-shadow: var(--glow-ember-soft);
    transition:
      box-shadow 500ms var(--ease-forge),
      border-color 500ms var(--ease-forge),
      transform 150ms var(--ease-forge);
  }
  .orb:hover {
    box-shadow: var(--glow-ember);
    border-color: color-mix(in srgb, var(--ember) 70%, transparent);
  }
  .orb:active {
    transform: scale(0.97);
  }
  .orb.recording {
    background: var(--ember);
    color: var(--surface);
    box-shadow: var(--glow-ember-strong);
    /* Animate transform only — animating the blurred shadow repaints the
       whole region every frame and judders scrolling in WebKitGTK. */
    animation: orb-breathe 2.4s var(--ease-forge) infinite;
  }
  .orb.busy {
    cursor: default;
    opacity: 0.85;
  }
  .orb:focus-visible {
    outline: 2px solid rgba(255, 122, 26, 0.6);
    outline-offset: 4px;
  }

  @keyframes orb-breathe {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(1.035); }
  }

  .spinner {
    width: 28px;
    height: 28px;
    border-radius: 999px;
    border: 3px solid color-mix(in srgb, var(--ember) 25%, transparent);
    border-top-color: var(--ember);
    animation: spin 0.9s linear infinite;
  }

  .stage-side {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .wave {
    transition: opacity 500ms var(--ease-forge);
  }
  .wave.dim {
    opacity: 0.55;
  }

  .stage-meta {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 30px;
  }

  .elapsed {
    font-family: var(--font-mono);
    font-size: 20px;
    font-weight: 600;
    color: var(--ember);
    font-variant-numeric: tabular-nums;
  }

  .muted-line {
    font-size: 13px;
    color: var(--muted);
  }

  .error-line {
    font-size: 13px;
    color: var(--bad);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .segments {
    padding: 18px 20px;
  }

  .segment-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
  }

  .segment-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .segment-row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    align-items: start;
    gap: 12px;
    padding: 10px 0;
    border-top: 1px solid var(--hairline);
  }
  .segment-row:first-child {
    border-top: 0;
  }
  .segment-row.failed .segment-text {
    color: var(--bad);
  }

  .segment-status {
    font-family: var(--font-mono);
    font-size: 10px;
    text-transform: uppercase;
    color: var(--muted);
  }
  .segment-status.pending {
    color: var(--ember);
  }

  .segment-text {
    min-width: 0;
    font-size: 13.5px;
    line-height: 1.55;
    color: color-mix(in srgb, var(--text) 90%, transparent);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }

  @media (max-width: 960px) {
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  .stat {
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .stat-value {
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }

  .ember-text {
    color: var(--ember);
  }

  .stat-sub {
    font-size: 12px;
    color: var(--muted);
  }

  .chart {
    padding: 20px;
  }

  .chart-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 14px;
  }

  .chart-total {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--muted);
  }

  .bars {
    display: grid;
    grid-template-columns: repeat(14, minmax(0, 1fr));
    gap: 8px;
    align-items: end;
  }

  .bar-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .bar-track {
    width: 100%;
    height: 72px;
    display: flex;
    align-items: flex-end;
  }

  .bar {
    width: 100%;
    border-radius: 4px 4px 2px 2px;
    background: linear-gradient(180deg, var(--ember-soft), var(--ember-deep));
    transition: height 600ms var(--ease-forge);
  }
  .bar.empty {
    background: var(--wash-strong);
  }

  .bar-label {
    font-family: var(--font-mono);
    font-size: 9px;
    text-transform: uppercase;
    color: var(--muted);
  }

  .last {
    padding: 20px;
  }

  .last-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 14px;
  }

  .last-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
  }

  .transcript {
    border-radius: 10px;
    padding: 14px 16px;
  }
  .transcript.raw {
    background: var(--well);
    border: 1px solid var(--well-border);
  }
  .transcript.clean {
    background: color-mix(in srgb, var(--ember) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--ember) 30%, transparent);
  }

  .transcript-label {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--muted);
    margin-bottom: 8px;
  }

  .transcript-text {
    font-size: 13.5px;
    line-height: 1.6;
    color: color-mix(in srgb, var(--text) 90%, transparent);
    max-height: 130px;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
