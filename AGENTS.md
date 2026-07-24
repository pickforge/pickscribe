# AGENTS

Repo-local guide for agents working in PickScribe — local dictation for Linux
(Tauri v2: Rust + whisper.cpp backend, Svelte 5 frontend, bun).

## Commands

- `bun install` then `bun run tauri dev` to develop.
- `bun run check` type-checks the Svelte frontend (`build` alone is just
  `vite build`); `bun run test` runs frontend unit tests; `bun run
  test:coverage` enforces the frontend coverage ratchet; `cargo test
  --workspace --locked --all-targets` covers the Rust side (the root package
  and `src-tauri` are one workspace, matching CI). Run these before calling
  work done.
- Default to tests with behavior changes: add a failing regression test for bugs
  when practical, characterize existing behavior before risky refactors, and
  keep tests in the same PR as the code they cover.
- Coverage gates are ratchets, not aspirations. Do not lower thresholds without
  maintainer approval.
- Durable business/domain behavior belongs in existing core/lib layers instead
  of UI components; keep it simple, without DDD ceremony.

## Invariants

- Audio is transcribed locally (whisper.cpp). Only the cleanup step may call
  the configured LLM endpoint, and local-only mode must keep working. Never
  widen what leaves the machine without updating README's privacy section.
- Follow the Pickforge design system: ember `#FF7A1A` accent, Geist/Geist Mono,
  tokens over raw values.
- When command discovery returns a resolved path, spawn that path instead of the
  bare command so desktop `PATH` fallbacks keep working.
- Test fakes must mirror the real implementation's error contract so tests do
  not pin fake-only behavior.
- Any spawned recording or capture child must have an owner with `Drop` cleanup
  so app exit cannot orphan it.

## Releasing

- Keep [`docs/releases/UNRELEASED.md`](docs/releases/UNRELEASED.md) current on
  PRs with user-facing or release-relevant changes. Track user-facing changes,
  internal/release changes, what was tested, what was not tested yet, and known
  blockers. At release time, copy and polish it into the GitHub release
  description, then reset the draft.
- Bump the version in `src-tauri/tauri.conf.json` and `package.json`, land on
  `main`, tag `vX.Y.Z`, and push the tag. CI builds the Linux bundles (deb +
  AppImage), signs the updater artifacts, generates `latest.json` (AppImage is
  the primary updater artifact), and uploads them to a draft release.
- Desktop-smoke the exact draft AppImage before publishing manually. A rerun
  replaces draft assets, so smoke the replacement AppImage again.
- The GitHub release description is the single source of release notes; polish
  it before publishing. pickforge.dev/pickscribe shows the latest published
  release via the GitHub API — no website change needed for a normal release.
- Only touch `landing-page` (`src/pages/products.ts`) when install methods,
  platforms, or positioning change.
## Workspace policy

For substantial work, read `../AGENTS.md` (workspace root) and use the `plan-issue` workflow — GitHub Issues are the canonical plan/progress tracker.
