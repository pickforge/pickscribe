# AGENTS

Repo-local guide for agents working in PickScribe — local dictation for Linux
(Tauri v2: Rust + whisper.cpp backend, Svelte 5 frontend, bun).

## Commands

- `bun install` then `bun run tauri dev` to develop.
- `bun run build` type-checks and builds the frontend; `cargo test` covers the
  Rust side. Run both before calling work done.

## Invariants

- Audio is transcribed locally (whisper.cpp). Only the cleanup step may call
  the configured LLM endpoint, and local-only mode must keep working. Never
  widen what leaves the machine without updating README's privacy section.
- Follow the Pickforge design system: ember `#FF7A1A` accent, Geist/Geist Mono,
  tokens over raw values.

## Releasing

- Bump the version in `src-tauri/tauri.conf.json` and `package.json`, land on
  `main`, tag `vX.Y.Z`, push the tag. CI builds the Linux bundles (deb +
  AppImage), signs the updater artifacts, generates `latest.json` (AppImage is
  the primary updater artifact), and publishes the release **live** — make
  sure `main` is ready before tagging.
- The GitHub release description is the single source of release notes; polish
  it right after the workflow finishes. pickforge.dev/pickscribe shows the
  latest release via the GitHub API — no website change needed for a normal
  release.
- Only touch `landing-page` (`src/pages/products.ts`) when install methods,
  platforms, or positioning change.
