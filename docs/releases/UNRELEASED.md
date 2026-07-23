# Unreleased

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- Transcribe audio or video files by dropping them onto the app or browsing
  from the dashboard. PickScribe runs local whisper.cpp transcription with
  progress and cancellation, stores results in History, and exports TXT, SRT,
  or VTT. AI cleanup remains off by default. File transcription requires
  `ffmpeg`.
- File transcriptions are excluded from dictation session and time-saved
  metrics.
- Slow cleanup endpoints, doctor checks, recording cancellation, and recorder
  warm-up no longer freeze the UI. Stopping incremental transcription now
  preserves completed segments and only re-transcribes the unfinished tail.
- PickScribe now finds `whisper-cli` in `~/.local/bin` when desktop sessions omit
  that directory from `PATH`.
- Double-clicking empty titlebar space now maximizes or restores the window.
- On Linux, the float capsule now keeps its glow visible without capturing
  clicks in the transparent margin. Other platforms keep a snug window to
  avoid intercepting clicks. Its idle waveform also re-measures correctly while
  the window settles.
- Tray state changes no longer repeatedly spawn desktop-theme probes.
- Settings now change runtime behavior only after persistence succeeds.
  Float-button changes from the tray or capsule stay synchronized with an open Settings form
  without overwriting unrelated unsaved edits.
- Fixed the float capsule staying out of KDE's Alt+Tab switcher when running
  through the `PICKSCRIBE_X11=1` XWayland fallback. The KWin `skipswitcher`
  rule now installs for any KDE Wayland session — native Wayland or its
  XWayland fallback — instead of only native Wayland.
- Fixed unsaved Settings actions disappearing at compact window sizes. The
  save/discard controls now render at the app overlay layer instead of
  inside the scrolling Settings surface (whose entrance animation made it a
  `position: fixed` containing block), so they stay visible and unclipped at
  every supported window size. The header Save button is now visible and
  disabled while clean, and hidden without shifting layout while dirty;
  exactly one primary Save action is presented at a time. The dirty-state
  overlay shows the full unsaved-indicator/Discard/Save dock at wide widths
  and a labeled Save pill (discard remains reachable via the existing
  navigation-away guard) at compact widths, matching the app's existing
  700px sidebar breakpoint.

## Internal/release changes

- Added file-transcription conversion, segmented whisper output, progress,
  TXT/SRT/VTT formatting, and additive history fields for source files and
  segments.
- The release workflow strips bundled `libwayland` libraries from AppImage
  builds, verifies the rebuilt image, and re-signs it before asset collection.
- Tagged builds create a draft GitHub release. Publishing remains a manual gate
  after the draft AppImage artifact passes a desktop smoke test.
- CLI and desktop cleanup now share provider resolution, local-only enforcement,
  prompts, response handling, raw fallback, and conservative segment safety.
- CLI and desktop dictation now share the Linux clipboard, paste-chord, and
  typing-backend delivery strategy.
- CI now blocks complexity regressions, coverage regressions, leaked secrets,
  and high or critical dependency advisories.

## Validation

### Tested

- Issue #46 (float capsule KDE Alt+Tab exclusion, porting the accepted
  PickGauge #49 fix merged as pickgauge#66): extracted the gating logic in
  `kwin.rs` into a pure `is_kde_wayland_session(xdg_session_type,
  xdg_current_desktop)` predicate and removed the `GDK_BACKEND == "x11"`
  early return that skipped the KWin rule for the `PICKSCRIBE_X11=1`
  XWayland fallback (renderer backend is orthogonal to session type — the
  KDE-desktop filter that protects non-KDE desktops from kwinrulesrc writes
  is unchanged). Ported 7 unit tests from pickgauge's `kwin.rs` covering the
  predicate and the existing `group_has_key` helper (verified with an
  isolated `rustc` compile — 7/7 pass — because the full
  `cargo test --workspace --locked --all-targets` run fails to build on this
  macOS dev machine identically on unmodified `origin/main`: `.transparent()`
  is not available without the tauri `"transparent"` Cargo feature here, a
  pre-existing environment gap unrelated to this change). `bun run check`,
  `bunx vitest run` (33 tests), and `bun run build` all pass. Updated the
  `ensure_float_window` doc comment and README's task-switcher table to
  match PickScribe's actual platform support (Linux only for now; Windows/
  macOS rows omitted rather than claiming untested support that doesn't
  exist yet). Real KDE Alt+Tab validation defers to Elberte-PC.
- Issue #45 (unsaved Settings actions at compact sizes, porting the accepted
  PickGauge #47 pattern): `bun run check`, `vitest run` (33 frontend tests,
  including a new `settingsSaveDisplayState` characterization suite covering
  clean/dirty header-visibility and single-action rules), `vitest run
  --coverage` (ratchet holds), and `bun run build`. PickScribe has no
  browser-preview/Playwright harness (unlike PickGauge's
  `scripts/validate-browser-preview.mjs`), so no headless-browser regression
  at the compact repro sizes was added — flagged as a follow-up decision
  rather than introducing new test tooling unrequested. Confirmed the same
  `.content.fade-up` containing-block hazard exists in PickScribe's
  `App.svelte` as in PickGauge, which the fix addresses the same way.
- Feature and fix PRs ran their focused frontend and Rust checks before merge.
- Shared cleanup policy: `cargo test --workspace --locked --all-targets`,
  `bun run build`, and CLI smoke checks for raw output, conservative segments,
  local-only fallback, and strict non-zero exit behavior.
- Shared Linux delivery: `cargo test --workspace --locked --all-targets`,
  `bun run build`, and CLI smoke checks for stdout-only, auto-to-type fallback,
  `~/.local/bin` backends, clipboard-required hotkeys, terminal paste chords,
  failed-copy suppression, and non-fatal paste failure.
- v0.2.0 release prep: `cargo check --workspace`, `bun run check`,
  `bun run test`, `bun run test:coverage`,
  `cargo test --workspace --locked --all-targets`, and
  `bun run pickforge-tauri-release validate-config`.
- CI gate installation: `bun run check`, `bun run lint`, `bun run
  test:coverage`, `bun run build`, root-package Rust tests, clippy, and
  llvm-cov, gitleaks, OSV-Scanner, and actionlint.

### Not tested yet

- v0.2.0 tagged release build and signed artifacts.
- Draft AppImage desktop smoke test, including interactive file drag/drop and
  dialog transcription.
- Installer and updater flow against the v0.2.0 draft artifacts.
- Workflow lint with `actionlint` (not installed locally).

### Release blockers

- Do not publish the draft GitHub release until its AppImage artifact passes a
  desktop smoke test.
