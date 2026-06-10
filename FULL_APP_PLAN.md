# PickScribe App Plan

## Current MVP status

We currently have a working standalone MVP:

```text
KDE shortcut / terminal command
  -> pickscribe-gui
  -> start/stop microphone recording with PipeWire pw-record
  -> local whisper.cpp STT
  -> DeepSeek cleanup via pickscribe-cleanup
  -> clipboard + ydotool typing
```

Current components:

- `pickscribe`: Rust standalone toggle command.
- `pickscribe-gui`: GUI/shortcut-safe wrapper that loads `~/.config/pickscribe/env`.
- `pickscribe-cleanup`: Rust text cleanup + clipboard/type helper.
- `pickscribe-cleanup-gui`: GUI/shortcut-safe cleanup wrapper.
- Local `whisper.cpp` installed in `~/.local/src/whisper.cpp`.
- Local multilingual Whisper model at `~/.local/share/whisper.cpp/models/ggml-base.bin`.
- DeepSeek official API cleanup using `deepseek-v4-flash` by default.
- Safe weekly whisper.cpp update checks via `PICKSCRIBE_AUTO_UPDATE_WHISPER="check"`.

## Recommended shortcut

Best default:

```text
Caps Lock remapped to F13/F20 -> /home/dev/.local/bin/pickscribe-gui
```

Why:

- It feels like a dedicated dictation key.
- It avoids common conflicts with editor shortcuts.
- It works well with toggle-to-record: press once to start, press again to stop.

Easy alternative:

```text
Meta/Super + Space -> /home/dev/.local/bin/pickscribe-gui
```

Caveat: `Meta + Space` can conflict with KDE shortcuts, keyboard layout switching, or launchers. If it is already used, prefer remapped Caps Lock or `Ctrl + Alt + Space`.

## Phase 1: Stabilize the MVP

Goal: make the current CLI flow reliable for daily use.

Tasks:

- [ ] Add clearer terminal/log output for each stage: recording, transcribing, cleaning, typing.
- [ ] Add `pickscribe doctor` command to check dependencies:
  - `pw-record`
  - `ydotool`
  - `ydotool.service`
  - `whisper-cli`
  - model file
  - DeepSeek key/config
- [ ] Add `pickscribe config` command to print non-secret active config.
- [ ] Better error handling when DeepSeek is down or slow.
- [ ] Add fallback behavior:
  - if DeepSeek fails, paste raw transcript
  - if typing fails, copy to clipboard only
- [ ] Add optional desktop notifications for failures and success.
- [ ] Add a small test script for end-to-end smoke testing.

## Phase 2: Better transcription quality and latency

Goal: improve accuracy and speed for English + Brazilian Portuguese.

Tasks:

- [ ] Benchmark `tiny`, `base`, `small`, and `large-v3-turbo` models.
- [ ] Add config presets:
  - fast
  - balanced
  - accurate
- [ ] Add `pickscribe install-model <name>`.
- [ ] Add model update/download verification with checksums where possible.
- [ ] Add optional initial Whisper prompt for English/PT-BR context.
- [ ] Add optional language lock:
  - `auto`
  - `en`
  - `pt`
- [ ] Consider GPU/Vulkan whisper.cpp build once stable, but keep CPU as fallback.

## Phase 3: Make it feel like a real app

Goal: remove terminal dependence and make usage polished.

Tasks:

- [x] Add v2 Pickforge Studio brand assets under `assets/branding/`.
- [x] Add tray app (Tauri 2 + Svelte 5 desktop app in `src-tauri/` + `src/`).
- [x] Show tray state (idle / recording / transcribing / cleaning via icon + tooltip).
- [x] Add GUI with Pickforge design system (dashboard, history, settings).
- [x] Add transcription history (SQLite, raw + cleaned text, search, delete).
- [x] Add floating always-on-top draggable button with live waveform
      (click opens the app, right-click toggles dictation).
- [x] Add user metrics (words, speaking time, sessions, estimated time saved
      vs typing, 14-day activity chart).
- [x] Add sound effects for start/stop/error (synthesized chimes, replaces
      notifications in the app flow).
- [x] Add recording duration indicator (dashboard timer).
- [x] Add cancel command from tray/menu and dashboard.
- [x] Global hotkey via `pickscribe-app --toggle` (single-instance forwarding).
- [ ] Add Caps Lock -> F13/F20 setup helper/documentation.
- [ ] Add push-to-talk mode in addition to toggle mode.
- [ ] Add simple logs view.

## Phase 4: Native audio recording

Goal: stop relying on external `pw-record`.

Tasks:

- [ ] Evaluate Rust audio capture libraries:
  - `cpal`
  - PipeWire bindings
- [ ] Implement direct microphone capture.
- [ ] Add audio device selection.
- [ ] Add automatic sample-rate conversion to 16 kHz mono.
- [ ] Add VAD/silence detection.
- [ ] Add auto-stop after silence.
- [ ] Add max recording duration safety limit.

## Phase 5: Native Whisper integration

Goal: reduce external process calls and improve control.

Options:

1. Keep `whisper.cpp` CLI for simplicity.
2. Use `whisper-rs` / direct whisper.cpp bindings.
3. Run a local STT server process.

Tasks:

- [ ] Benchmark CLI overhead vs embedded library.
- [ ] Decide whether embedded Whisper is worth the complexity.
- [ ] If embedded:
  - load model once
  - keep daemon warm
  - reduce per-recording latency
- [ ] Keep CLI backend as fallback.

## Phase 6: Configuration system

Goal: provide a clean config file instead of many env vars.

Possible config path:

```text
~/.config/pickscribe/config.toml
```

Example:

```toml
[hotkey]
mode = "toggle"

[stt]
backend = "whisper-cli"
model = "base"
language = "auto"

[cleanup]
provider = "deepseek"
model = "deepseek-v4-flash"
timeout_secs = 30

[paste]
backend = "ydotool"
copy_to_clipboard = true
paste_method = "hotkey"
paste_chord = "ctrl-v"
paste_delay_ms = 250

[updates]
whisper_cpp = "check"
interval_hours = 168
```

Tasks:

- [ ] Add TOML config parser.
- [ ] Migrate env vars to config file.
- [ ] Keep env vars as overrides.
- [ ] Add `pickscribe config init`.
- [ ] Add `pickscribe config show` with secrets redacted.

## Phase 7: Packaging and installation

Goal: make setup repeatable and easy.

Tasks:

- [ ] Add one-command installer.
- [ ] Add system dependency detection for Arch/CachyOS.
- [ ] Add optional `.desktop` file.
- [ ] Add systemd user service for daemon/tray mode.
- [ ] Add uninstall command/script.
- [ ] Create release binaries with GitHub Actions.
- [ ] Package as AppImage or distro package later.

## Phase 8: Privacy and provider options

Goal: support local, BYOK, and cloud modes cleanly.

Cleanup providers:

- [x] DeepSeek official API.
- [x] Ollama local/cloud OpenAI-compatible endpoint.
- [x] OpenAI-compatible custom endpoint (with `/models` discovery in the app).
- [x] No cleanup / raw transcript.

STT providers:

- [x] Local whisper.cpp.
- [ ] Optional cloud STT later.

Security tasks:

- [ ] Never print API keys.
- [ ] Redact secrets from logs.
- [ ] Document exactly what text/audio leaves the machine.
- [x] Add local-only mode (app setting; blocks non-loopback cleanup endpoints).

## Phase 9: Quality-of-life features

Potential features:

- [ ] Dictation history with opt-in local storage.
- [ ] Per-app behavior:
  - terminal-aware paste chord selection (`Ctrl+V` vs `Ctrl+Shift+V`)
  - paste raw in terminal
  - clean prose in browser/chat
  - code-friendly mode in editors
- [ ] Voice commands:
  - “new paragraph”
  - “comma”
  - “period”
  - “delete that”
- [ ] Prompt profiles:
  - casual
  - professional
  - concise
  - Portuguese BR
  - coding
- [ ] Translation mode.
- [ ] Clipboard-only mode.
- [ ] Manual review popup before paste.

## Suggested next development order

1. Add `pickscribe doctor`.
2. Add config file support.
3. Add model presets and installer commands.
4. Add tray/daemon mode.
5. Add native audio capture or warm Whisper backend.
6. Package releases.

## Definition of “good app”

The project becomes a good daily-driver app when:

- Setup takes less than 5 minutes.
- A global shortcut works reliably after reboot.
- Start/stop feedback is obvious.
- English and Brazilian Portuguese work well.
- Failures gracefully fall back to clipboard/raw transcript.
- Config is inspectable without exposing secrets.
- Updating local Whisper is one command or safe automatic notification.
- The app does not require Whispering or manual terminal use.
