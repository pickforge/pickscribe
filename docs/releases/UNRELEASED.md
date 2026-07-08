# Unreleased

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- Adopted the Pickforge Studio shared chrome: a frameless main window with a
  38px titlebar (brand mark + wordmark, live dictation-stage status pill, and
  platform-aware minimize/maximize/close controls with edge/corner resize
  handles) replacing the native GTK titlebar and the decorative dots header.
- Switched the blinking status dots to the bracket motif (bracket status
  indicators, `[` section-eyebrow ticks, ember bracket ticks for unsaved
  changes) and unified the bottom bar to the shared 24px statusbar.
- Fixed the stray horizontal scrollbar that could appear along the window
  bottom.
- Added an in-app platform gate: Linux remains the supported release target,
  while macOS and Windows show the native work still required before release.
- Updated install and release messaging so PickScribe no longer claims
  native-host smoke checks are automated.
- Linux curl installs now use a rootless AppImage wrapper that falls back on
  FUSE3-only systems and installs a launcher icon/menu entry.
- Fixed the floating capsule waveform alignment.
- Added opt-in segment cleanup controls for incremental dictation in Settings
  and the legacy CLI.
- Made partial segment cleanup conservative so instruction examples and
  boilerplate are ignored instead of appearing in the live transcript.

## Internal/release changes

- Added repo-local release tracking in `docs/releases/UNRELEASED.md`.
- Added a shared platform-support contract used by the app doctor, dictation
  start guard, dashboard state, and release docs.
- Added installer smoke tests for AppImage desktop integration and symlink-safe
  upgrades.
- Added the disabled incremental dictation core foundation: segment/session
  assembly, WAV segment slicing, and mocked runner safeguards.
- Wired the Tauri app incremental dictation path behind the disabled
  `[incremental]` flag, including live segment state, cancellation guards, and
  full-WAV fallback.
- Added opt-in legacy CLI incremental transcription via `--incremental` or
  `PICKSCRIBE_INCREMENTAL_DICTATION=1`, with final full-WAV fallback.
- Kept segment cleanup separate from incremental transcription and disabled by
  default; final paste/history still use one final cleanup pass.

## Validation

### Tested

- Reviewed the release tracking docs.
- `bun run check`
- `bun run test`
- `bun run test:coverage`
- `bun run build`
- `bun run pickforge-tauri-release validate-config`
- `cargo check --workspace --all-targets`
- `cargo check -p pickscribe-app --features pickscribe-app/custom-protocol`
- `cargo test --workspace --locked --all-targets`
- `cargo llvm-cov --workspace --locked --all-targets --fail-under-lines 17
  --lcov --output-path target/llvm-cov/lcov.info` with system LLVM tools
- `git diff --check`
- Standalone `rustfmt --edition 2024` on touched Rust files.
- Fake-recorder CLI incremental stop, cancel, active-STT cancel, and orphan
  worker smoke checks.
- Mocked CLI segment cleanup smoke checks for enabled cleanup, unsafe-mode
  gating, and slow cleanup cancel/nonblocking behavior.
- Segment cleanup guard unit tests for conservative edits, brand casing, and
  instruction-example leaks.
- `bun run test:installer`
- Browser preview of `/?window=float` at `208x60`

### Not tested yet

- `cargo fmt --all --check` because the active Cargo toolchain cannot find its
  `cargo-fmt` component in this environment.
- Updater flow.
- Native-host smoke tests.
- Live visual check of the frameless studio titlebar, window controls, and
  resize handles in a running WebKitGTK window (verified via checks/tests only).

### Release blockers

- macOS and Windows remain blocked until native audio capture, paste
  automation, global shortcuts, tray/window validation,
  signing/notarization/code-signing, and native-host smoke tests are done.
