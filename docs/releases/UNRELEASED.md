# Unreleased

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- None yet.

## Internal/release changes

- Release workflow now strips bundled `libwayland` libraries from AppImage
  builds and re-signs the AppImage before collecting release assets.
- File-transcription core (PR 1 of 2 for #30): ffmpeg→WAV conversion, whisper
  segment output with progress, TXT/SRT/VTT formatters, and an additive history
  DB migration (`source_file`, `segments_json`). No user-facing surface yet;
  the UI ships in PR 2.

## Validation

### Tested

- None yet.

### Not tested yet

- App build.
- Installer or updater flow.
- Platform smoke checks.

### Release blockers

- None known.
