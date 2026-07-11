# Unreleased

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- Double-clicking empty titlebar space now maximizes or restores the window.
- Transcribe a file: drop an audio or video file onto the app (or browse from
  the dashboard) to get a transcript from the local whisper.cpp engine, with
  live progress and cancel. Results land in History marked with the source
  file and export as TXT, SRT, or VTT. Fully local; the optional AI cleanup
  step is off by default for files. Requires `ffmpeg` (new doctor check).
- File transcriptions are excluded from the dictation metrics (sessions,
  minutes saved).
- whisper-cli installed in `~/.local/bin` is now found even when the app is
  launched from the app menu on sessions whose PATH omits that directory
  (#5).
- The float capsule's glow now fades out smoothly instead of being clipped
  into a hard rectangle by the window edge; the transparent margin around
  the capsule is click-through (#38).

- The float capsule's idle waveform no longer renders with stretched dashes
  on some app starts; it now re-measures itself as the window settles (#41).

## Internal/release changes

- Release workflow now strips bundled `libwayland` libraries from AppImage
  builds and re-signs the AppImage before collecting release assets.
- File-transcription core (PR 1 of 2 for #30): ffmpeg→WAV conversion, whisper
  segment output with progress, TXT/SRT/VTT formatters, and an additive history
  DB migration (`source_file`, `segments_json`). No user-facing surface yet;
  the UI ships in PR 2.

## Validation

### Tested

- `bun run test:coverage` (23 tests) and `bun run check` for the titlebar
  double-click fix.
- `cargo test --workspace` (79), `bun run check`, `bun run test:coverage`
  (ratchet green) on the file-transcription PRs.
- ffmpeg conversion flags verified live against WAV/MP3/MP4 samples; whisper
  `--output-json`/progress format verified against the installed whisper-cli.

### Not tested yet

- App build.
- Installer or updater flow.
- Platform smoke checks.
- Interactive drag-drop/dialog smoke in a real desktop session.

### Release blockers

- None known.
