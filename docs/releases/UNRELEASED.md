# Unreleased

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- Added an in-app platform gate: Linux remains the supported release target,
  while macOS and Windows show the native work still required before release.
- Updated install and release messaging so PickScribe no longer claims
  native-host smoke checks are automated.

## Internal/release changes

- Added repo-local release tracking in `docs/releases/UNRELEASED.md`.
- Added a shared platform-support contract used by the app doctor, dictation
  start guard, dashboard state, and release docs.

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
- `git diff --check`

### Not tested yet

- Installer or updater flow.
- Native-host smoke tests.

### Release blockers

- macOS and Windows remain blocked until native audio capture, paste
  automation, global shortcuts, tray/window validation,
  signing/notarization/code-signing, and native-host smoke tests are done.
