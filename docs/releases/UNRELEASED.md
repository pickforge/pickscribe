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

- Integrated `@pickforge/tauri-updater` behind the default-off
  `studioUpdateDialog` release flag. The shared dialog preserves packaged-only,
  visible-main-window startup checks while excluding hidden login starts and the
  floating capsule; the existing confirm flow remains active while the flag is
  off.
- Added deterministic development fixtures for the shared update dialog at
  `?update-fixture=available` and `?update-fixture=downloading`. PickScribe has
  no visual-regression harness, so this keeps visual review reproducible without
  introducing a new browser-test stack in this PR.
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
- Added macOS compile support for the transparent float window and a blocking
  macOS CI job that checks and tests the app on macOS 15.
- Added native macOS microphone capture via `ffmpeg`'s `avfoundation` input
  device (16kHz mono s16 WAV, matching the Linux PipeWire pipeline), reusing
  the existing recorder process/stop/log machinery instead of a separate
  CoreAudio backend. `SttConfig::recorder` now defaults to `ffmpeg` on macOS
  and `pw-record` on Linux; explicit user config is unaffected. Removed the
  "Native audio capture" release blocker for macOS (dictation as a whole
  remains blocked on paste automation, global shortcuts, and signing). The
  doctor page now probes the platform-appropriate recorder (`ffmpeg` on
  macOS, `pw-record` on Linux) before the dictation-support early return.
  Added `src-tauri/Info.plist` with `NSMicrophoneUsageDescription`, which
  Tauri 2 merges into the generated macOS bundle `Info.plist`.
- Added macOS text delivery through OS-shipped subprocesses: clipboard copy
  via `pbcopy`, and hotkey paste / typing via `osascript` (System Events),
  reusing the existing Linux `ProcessRuntime` seam with no new engine-crate
  dependencies. Auto delivery resolves to the paste-chord hotkey on macOS.
  Failures caused by a missing Accessibility grant now surface a clear
  message pointing at System Settings → Privacy & Security → Accessibility
  instead of the raw System Events error. Doctor's "Clipboard" and "Paste
  backend" checks are now platform-aware (`pbcopy` / Accessibility-trust on
  macOS, `wl-copy`/`xclip`/`xsel` / `ydotool`+socket on Linux, unchanged) and
  now run for macOS too. Removed the "Paste automation" release blocker for
  macOS; dictation still isn't a supported release target there pending the
  other blockers (native audio capture, global shortcuts, tray/window
  validation, signing, smoke tests).

## Validation

### Tested

- pickscribe#66 PR 2 (macOS audio capture via ffmpeg/avfoundation): `cargo
  test` (workspace root, 122 tests including new `recorder_args` and
  `platform` coverage), `cargo test --manifest-path src-tauri/Cargo.toml` (37
  tests), `cargo clippy --workspace --all-targets -- -D warnings` (matches CI).
  Verified device syntax on this machine with `ffmpeg -f avfoundation
  -list_devices true -i ""` and confirmed `:default` is accepted as the input
  target with this ffmpeg build (8.1). Live smoke: spawned the app's exact
  `ffmpeg -nostdin -hide_banner -f avfoundation -i :default -ar 16000 -ac 1
  -c:a pcm_s16le -y <path>` args, stopped with `SIGINT` after ~2s matching
  `Recording::stop`'s escalation path — ffmpeg exited normally ("received
  signal 2") and produced a valid 37,966-byte WAV, well above the 8KB
  too-short threshold. No macOS TCC microphone permission prompt appeared;
  Terminal already had mic authorization on this machine from an earlier
  `-list_devices` probe.
- pickscribe#66 PR 3 (macOS text delivery): `cargo test` (root, 108/108 +
  10/10 + 7/7 passing), `cargo test --manifest-path src-tauri/Cargo.toml`
  (37/37 passing), and `cargo clippy --workspace --all-targets -- -D
  warnings` (clean), all on macOS. Live smoke on this macOS machine: the
  real `ProcessRuntime::copy_to_clipboard` macOS path piped a marker string
  through `pbcopy` and read it back with `pbpaste` — round-tripped
  correctly (kept as an `#[ignore]`d regression test, run manually with
  `--ignored`). Did not exercise `osascript` keystroke synthesis (paste
  chord / typing) live, to avoid sending real keystrokes into the active
  session; also skipped the proposed harmless `osascript -e '... count
  processes'` probe because it would trigger a first-run System Events
  automation-permission dialog, which is itself a disturbance on a live
  machine — both are covered instead by unit tests against the
  `DeliveryRuntime` seam and the pure script-building/error-mapping
  functions.
- Issue #59 (macOS compile support and CI): `cargo check --manifest-path
  src-tauri/Cargo.toml`, `cargo test --manifest-path src-tauri/Cargo.toml` (37
  tests), `bun install --frozen-lockfile && bun run check`, `bun run lint`, and
  `bun run build` on macOS.
- pickforge-platform#36 PR 4 (default-off shared update dialog): `bun run
  check`, `bunx vitest run` (39 tests, including flag selection,
  visible/focused deferral, and hidden-autostart single-check coverage), `bun
  run test:coverage`, and `bun run build`. The full `bun run test` command is
  blocked before Vitest by the pre-existing `scripts/install.sh` line 154 shell
  syntax error, unchanged from `origin/main`.
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

- Shared update dialog visual fixture review at 1020×720 and 780×560, and the
  owner-gated signed packaged update smoke.
- v0.2.0 tagged release build and signed artifacts.
- Draft AppImage desktop smoke test, including interactive file drag/drop and
  dialog transcription.
- Installer and updater flow against the v0.2.0 draft artifacts.
- Workflow lint with `actionlint` (not installed locally).

### Release blockers

- Do not publish the draft GitHub release until its AppImage artifact passes a
  desktop smoke test.
