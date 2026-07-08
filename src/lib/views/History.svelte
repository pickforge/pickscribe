<script lang="ts">
  import Copy from "phosphor-svelte/lib/Copy";
  import Check from "phosphor-svelte/lib/Check";
  import Trash from "phosphor-svelte/lib/Trash";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import {
    api,
    desktopApiAvailable,
    formatDuration,
    formatError,
    formatTimestamp,
    type HistoryEntry,
  } from "../api";

  let { historyVersion }: { historyVersion: number } = $props();

  let entries = $state<HistoryEntry[]>([]);
  let search = $state("");
  let error = $state<string | null>(null);
  let loaded = $state(false);
  let expanded = $state<Set<number>>(new Set());
  let copiedId = $state<string | null>(null);
  let confirmClear = $state(false);

  $effect(() => {
    void historyVersion;
    if (!desktopApiAvailable()) {
      loaded = true;
      return;
    }
    const query = search;
    const timer = setTimeout(
      () => {
        api
          .listHistory(query, 200, 0)
          .then((list) => {
            entries = list;
            error = null;
            loaded = true;
          })
          .catch((err) => {
            error = formatError(err);
            loaded = true;
          });
      },
      query ? 200 : 0
    );
    return () => clearTimeout(timer);
  });

  function toggleExpanded(id: number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  async function copy(entry: HistoryEntry, kind: "raw" | "clean") {
    const text = kind === "clean" ? (entry.cleaned_text ?? entry.raw_text) : entry.raw_text;
    try {
      await api.copyText(text);
      copiedId = `${entry.id}-${kind}`;
      setTimeout(() => (copiedId = null), 1500);
    } catch (err) {
      error = formatError(err);
    }
  }

  async function remove(id: number) {
    try {
      await api.deleteHistoryEntry(id);
      entries = entries.filter((e) => e.id !== id);
    } catch (err) {
      error = formatError(err);
    }
  }

  async function clearAll() {
    if (!confirmClear) {
      confirmClear = true;
      setTimeout(() => (confirmClear = false), 4000);
      return;
    }
    try {
      await api.clearHistory();
      entries = [];
      confirmClear = false;
    } catch (err) {
      error = formatError(err);
    }
  }
</script>

<section class="history">
  <header class="head fade-up">
    <div>
      <p class="eyebrow ember pf-eyebrow-row"><span class="pf-eyebrow-tick"></span>§ 02 · History</p>
      <h2>Every dictation, before and after cleanup</h2>
    </div>
    {#if entries.length > 0}
      <button type="button" class="btn btn-danger btn-sm" onclick={clearAll}>
        <Trash size={13} />
        {confirmClear ? "Confirm clear all?" : "Clear all"}
      </button>
    {/if}
  </header>

  <div class="search-row">
    <span class="search-icon"><MagnifyingGlass size={15} /></span>
    <input
      class="input search-input"
      type="text"
      placeholder="Search transcriptions…"
      bind:value={search}
    />
  </div>

  {#if error}
    <p class="error-line">{error}</p>
  {/if}

  {#if loaded && entries.length === 0}
    <div class="empty card">
      <img src="/brand/pickscribe-mark-128.svg" alt="" width="56" height="56" />
      <h3>{search ? "Nothing matches that search" : "No dictations yet"}</h3>
      <p>
        {search
          ? "Try a different term — both raw and cleaned text are searched."
          : "Press your hotkey or the orb on the dashboard. Every transcription lands here, raw and cleaned."}
      </p>
    </div>
  {:else}
    <div class="list">
      {#each entries as entry (entry.id)}
        <article class="entry card">
          <div class="entry-head">
            <div class="entry-meta">
              <span class="entry-when">{formatTimestamp(entry.created_at)}</span>
              <span class="pill">{entry.word_count} words</span>
              <span class="pill">{formatDuration(entry.duration_ms)}</span>
              {#if entry.cleaned_text}
                <span class="pill ember">{(entry.provider || "cleaned").toUpperCase()}</span>
              {:else}
                <span class="pill">RAW ONLY</span>
              {/if}
            </div>
            <div class="entry-actions">
              <button
                type="button"
                class="btn btn-ghost btn-sm"
                onclick={() => copy(entry, "clean")}
                title="Copy text"
              >
                {#if copiedId === `${entry.id}-clean`}<Check size={13} /> Copied{:else}<Copy size={13} /> Copy{/if}
              </button>
              <button
                type="button"
                class="icon-btn"
                onclick={() => remove(entry.id)}
                title="Delete entry"
                aria-label="Delete entry"
              >
                <Trash size={14} />
              </button>
            </div>
          </div>

          <p class="entry-text">{entry.cleaned_text ?? entry.raw_text}</p>

          {#if entry.cleaned_text}
            <button type="button" class="raw-toggle" onclick={() => toggleExpanded(entry.id)}>
              <span class="caret" class:open={expanded.has(entry.id)}><CaretDown size={12} /></span>
              Raw whisper transcript
            </button>
            {#if expanded.has(entry.id)}
              <div class="raw-block">
                <p class="raw-text">{entry.raw_text}</p>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => copy(entry, "raw")}>
                  {#if copiedId === `${entry.id}-raw`}<Check size={13} /> Copied{:else}<Copy size={13} /> Copy raw{/if}
                </button>
              </div>
            {/if}
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .history {
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

  .search-row {
    position: relative;
  }

  .search-icon {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--muted);
    display: flex;
  }

  .search-input {
    padding-left: 36px;
  }

  .error-line {
    font-size: 13px;
    color: var(--bad);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 48px 24px;
    text-align: center;
  }

  .empty img {
    opacity: 0.8;
  }

  .empty h3 {
    font-size: 17px;
  }

  .empty p {
    font-size: 13px;
    color: var(--muted);
    max-width: 380px;
    line-height: 1.6;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .entry {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .entry-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .entry-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .entry-when {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--muted);
  }

  .entry-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }

  .icon-btn {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: none;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition: color 300ms var(--ease-forge), background 300ms var(--ease-forge);
  }
  .icon-btn:hover {
    color: var(--bad);
    background: rgba(251, 113, 133, 0.08);
  }

  .entry-text {
    font-size: 13.5px;
    line-height: 1.6;
    color: color-mix(in srgb, var(--text) 92%, transparent);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .raw-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    border: none;
    background: transparent;
    padding: 0;
    font-family: var(--font-mono);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.14em;
    color: var(--muted);
    cursor: pointer;
    transition: color 300ms var(--ease-forge);
  }
  .raw-toggle:hover {
    color: var(--text);
  }

  .caret {
    display: flex;
    transition: transform 300ms var(--ease-forge);
  }
  .caret.open {
    transform: rotate(180deg);
  }

  .raw-block {
    background: var(--well);
    border: 1px solid var(--well-border);
    border-radius: 10px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }

  .raw-text {
    font-size: 13px;
    line-height: 1.6;
    color: color-mix(in srgb, var(--text) 70%, transparent);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
