# Unreleased (v0.3.0 candidate)

Working draft for the next PickScribe release. Keep this current while PRs
land. At release time, copy and polish it into the GitHub release description,
then reset this file.

## User-facing changes

- PickScribe is now a lean daemon instead of the Tauri desktop app. The dashboard, settings window, tray, and in-app updater are gone.
- Tap Right Alt to dictate with Claude Haiku 5.5 cleanup, Shift + Right Alt for a raw transcript that stays on the machine, and Esc to cancel.
- A floating capsule shows the sound waves while listening and a pulse while thinking. It never takes focus.
- Whisper `large-v3-turbo` stays loaded on the GPU through Vulkan, and each recording picks the most likely of the configured languages.
- Settings move to `~/.config/pickscribe/pickscribe.toml`. The old `config.toml` and history database are left untouched.
- A dictionary of terms and corrections (`Bitforge -> Pickforge`) in `~/.config/pickscribe/dictionary.txt`, editable with `pickscribe dict` and by AI agents through `pickscribe mcp`.
- Recent dictations are kept in a local history, so text is not lost when no field had focus.
- Every 10 AI dictations, PickScribe looks for misheard words and suggests corrections. You add or ignore each one in the panel; nothing changes without your yes.
- On Omarchy, the capsule follows the theme colors, and a bar widget shows the state, history, and dictionary.
- Cleanup effort is configurable and defaults to `low`. The HTTPS connection opens while you speak.
- Cleanup rewrites a dictated question instead of answering it. If the model answers anyway, the raw text is pasted.

## Internal/release changes

- The release workflow, installer, and legacy shell wrappers were removed. `scripts/install` builds from source and installs a systemd user service.

## Validation

### Tested

- Unit tests for the shortcut tracker, settings, cleanup request and response, paste shortcut choice, resampling, the capsule animation and theme, the dictionary, history, status stream, and MCP server.
- Transcription of the whisper.cpp JFK sample on an RX 9070 through Vulkan.
- Daemon start, single-instance guard, capsule display on Hyprland, and cancel.
- Daily dictation with cleanup on Hyprland since 2026-10-18: 28 dictations, all pasted.

### Not tested yet

- Live dictation in English.
- Paste into terminals and Electron apps.

### Release blockers

- A new release pipeline for the daemon.
