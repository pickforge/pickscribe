# PickScribe

### Local Linux dictation with AI cleanup — by Pickforge

PickScribe is a Linux-first dictation tool that records your microphone, transcribes speech locally with `whisper.cpp`, cleans the transcript with an OpenAI-compatible LLM provider, and pastes the final text into the focused app.

```text
Shortcut
  -> record microphone
  -> local whisper.cpp speech-to-text
  -> DeepSeek V4 Flash cleanup with thinking disabled
  -> clipboard + paste hotkey into the focused field
```

> **Status:** working Rust MVP for CachyOS/Arch + KDE/Wayland. The CLI flow works today; tray UI, native audio capture, config UI, and packaging are planned in [`FULL_APP_PLAN.md`](FULL_APP_PLAN.md).

## Current behavior

- Toggle recording with one command: press once to start, press again to stop.
- Transcribes locally with `whisper.cpp` and a multilingual model.
- Supports English and Portuguese; Portuguese cleanup is instructed to use Brazilian Portuguese.
- Cleans text through the official DeepSeek API using `deepseek-v4-flash`.
- Sends `thinking: { "type": "disabled" }` for low-latency cleanup.
- Copies the cleaned text to clipboard and pastes with `Ctrl+V` by default.
- Keeps old MVP aliases (`voice-flow`, `voice-cleanup`) for compatibility, but the app name is PickScribe.

## Quick start

Install system dependencies on Arch/CachyOS:

```bash
sudo pacman -S --needed rust cargo git cmake ninja ffmpeg pipewire-audio wl-clipboard ydotool gst-plugins-good
```

Enable `ydotool` for Wayland paste automation:

```bash
sudo usermod -aG input "$USER"
systemctl --user enable --now ydotool.service
```

Log out and back in after changing groups.

Build and install PickScribe:

```bash
cargo build --release --bins
mkdir -p ~/.local/bin
cp target/release/pickscribe target/release/pickscribe-cleanup ~/.local/bin/
cp scripts/pickscribe-env.sh scripts/pickscribe-gui scripts/pickscribe-cleanup-gui ~/.local/bin/
cp scripts/voice-flow scripts/voice-cleanup scripts/voice-flow-gui scripts/voice-cleanup-gui scripts/install-whisper-cpp-local ~/.local/bin/
chmod +x ~/.local/bin/pickscribe* ~/.local/bin/voice-flow ~/.local/bin/voice-cleanup ~/.local/bin/voice-flow-gui ~/.local/bin/voice-cleanup-gui ~/.local/bin/install-whisper-cpp-local
```

Install/update local `whisper.cpp` and the multilingual `base` model:

```bash
install-whisper-cpp-local
```

## DeepSeek setup

Create the GUI-safe config file:

```bash
mkdir -p ~/.config/pickscribe
chmod 700 ~/.config/pickscribe
nano ~/.config/pickscribe/env
chmod 600 ~/.config/pickscribe/env
```

Recommended contents:

```bash
DEEPSEEK_API_KEY="your_api_key_here"

PICKSCRIBE_PROVIDER="deepseek"
PICKSCRIBE_MODEL="deepseek-v4-flash"
PICKSCRIBE_ENDPOINT="https://api.deepseek.com/v1/chat/completions"
PICKSCRIBE_DEEPSEEK_THINKING="disabled"

PICKSCRIBE_LANGUAGE="auto"
PICKSCRIBE_WHISPER_MODEL="$HOME/.local/share/whisper.cpp/models/ggml-base.bin"
PICKSCRIBE_WHISPER_MODEL_NAME="base"

PICKSCRIBE_PASTE_METHOD="hotkey"
PICKSCRIBE_PASTE_CHORD="ctrl-v"
PICKSCRIBE_PASTE_DELAY_MS="250"

PICKSCRIBE_AUTO_UPDATE_WHISPER="check"
PICKSCRIBE_UPDATE_INTERVAL_HOURS="168"
```

`deepseek-v4-flash` is the recommended model for dictation cleanup. PickScribe disables DeepSeek thinking/reasoning mode because cleanup should be fast and concise.

## Testing

Safe terminal test with no paste:

```bash
pickscribe-gui start --no-notify
# speak
pickscribe-gui stop --stdout-only --no-notify
```

Copy-only test:

```bash
pickscribe-gui start --no-notify
# speak
pickscribe-gui stop --no-paste --print --no-notify
```

Normal use from a shortcut:

```bash
pickscribe-gui
# speak
pickscribe-gui
```

Do not use the normal paste flow from a terminal unless you intentionally want the final text pasted back into that terminal. The normal flow pastes into whichever app is focused.

## Recommended hotkey

### Practical default

Use:

```text
Ctrl + Alt + Space
```

Bind it to:

```bash
/home/dev/.local/bin/pickscribe-gui
```

KDE path:

```text
System Settings -> Keyboard -> Shortcuts -> Add New -> Command or Script
```

### Best long-term hotkey: Caps Lock remapped to F13/F20

If you want Caps Lock as the PickScribe key, do **not** bind raw Caps Lock directly while it still toggles capitalization. Instead, remap Caps Lock to a harmless key such as `F13` or `F20`, then bind that key to PickScribe.

Why this avoids conflicts:

- Caps Lock no longer toggles uppercase.
- Apps see a dedicated unused function key instead of Caps Lock.
- KDE binds the remapped key to PickScribe normally.

Recommended future setup:

```text
Caps Lock -> F20 -> /home/dev/.local/bin/pickscribe-gui
```

A robust Wayland-friendly way to do this is a low-level remapper such as `keyd` or `input-remapper`. Once remapped, assign the resulting F13/F20 key in KDE Shortcuts.

## Terminal paste behavior

Most GUI apps paste with:

```text
Ctrl + V
```

Most Linux terminals paste with:

```text
Ctrl + Shift + V
```

PickScribe defaults to `Ctrl+V` because normal text fields are the main target:

```bash
PICKSCRIBE_PASTE_CHORD="ctrl-v"
```

For terminal-focused dictation, either temporarily run:

```bash
PICKSCRIBE_PASTE_CHORD="ctrl-shift-v" pickscribe-gui
```

or create a second KDE shortcut named `PickScribe Terminal` with this command:

```bash
bash -lc 'PICKSCRIBE_PASTE_CHORD="ctrl-shift-v" /home/dev/.local/bin/pickscribe-gui'
```

Automatic terminal detection is planned, but KDE/Wayland does not expose the active native window class through simple `xdotool` in this setup.

## Updating local whisper.cpp

Because the recommended setup builds `whisper.cpp` under `~/.local/src`, pacman/yay will not update it automatically.

Check for updates:

```bash
pickscribe-gui check-whisper
```

Update/rebuild whisper.cpp and relink `~/.local/bin/whisper-cli`:

```bash
pickscribe-gui update-whisper
```

Safe automatic update checks are enabled with:

```bash
PICKSCRIBE_AUTO_UPDATE_WHISPER="check"
PICKSCRIBE_UPDATE_INTERVAL_HOURS="168"
```

Use `install` instead of `check` only if you want the first recording after an upstream update to pull/rebuild automatically.

## CLI reference

Main flow:

```bash
pickscribe --help
pickscribe start
pickscribe stop --stdout-only
pickscribe cancel
pickscribe check-whisper
pickscribe update-whisper
```

Cleanup helper:

```bash
pickscribe-cleanup --help
echo "hello this needs punctuation" | pickscribe-cleanup --stdout-only
```

Legacy aliases remain available during the MVP:

```bash
voice-flow-gui
voice-cleanup-gui
voice-flow
voice-cleanup
```

## Privacy and security

- Audio transcription is local when using the bundled `whisper.cpp` flow.
- Transcript text is sent to DeepSeek only for cleanup when LLM cleanup is enabled.
- API keys live in `~/.config/pickscribe/env`, which should be `chmod 600`.
- PickScribe never intentionally prints API keys; docs and diagnostics should redact secrets.

## Roadmap

See [`FULL_APP_PLAN.md`](FULL_APP_PLAN.md) for the full product plan, including:

- dependency doctor command;
- config file migration;
- model presets;
- Caps Lock/F13 setup helper;
- terminal auto-detection;
- tray/daemon mode;
- native audio capture;
- embedded/warm Whisper backend;
- release packaging.

## License

MIT © 2026 Pickforge
