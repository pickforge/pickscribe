<p align="center">
  <img src="assets/branding/pickscribe-lockup-horizontal.svg" alt="PickScribe" width="560">
</p>

# PickScribe

Dictation for Linux. Tap Right Alt, speak, and tap it again. PickScribe transcribes your speech on your own GPU, cleans the text with Claude, and pastes it into the app you are using.

A small capsule floats at the bottom of the screen while you dictate. It shows the sound waves while it listens and a moving pulse while it thinks. It never takes keyboard focus, and clicks pass through it.

| Shortcut | What it does |
| --- | --- |
| Right Alt | Start or stop dictation. The text goes through AI cleanup. Ember bars. |
| Shift + Right Alt | Start or stop dictation. The raw transcript is pasted and never leaves your machine. White bars. |
| Esc | Discard the current recording. |

Right Alt only counts when you tap it alone, so AltGr characters and Alt shortcuts keep working.

## Requirements

PickScribe targets Wayland. It reads the focused window from Hyprland to choose the paste shortcut; on other compositors it always pastes with Ctrl+V.

On Arch Linux:

```bash
sudo pacman -S --needed rust cmake clang shaderc vulkan-headers vulkan-icd-loader gtk4 gtk4-layer-shell pipewire alsa-lib wl-clipboard wtype
sudo usermod -aG input "$USER"
```

Log out and back in after joining the `input` group. PickScribe reads Right Alt directly from the keyboard, so the shortcut needs no compositor setup.

You also need a Vulkan driver for your GPU, such as `vulkan-radeon` for AMD.

## Install

```bash
scripts/install
```

The script builds PickScribe, downloads the Whisper `large-v3-turbo` model (1.6 GB, checked against its SHA-256), installs `~/.local/bin/pickscribe`, and starts it as the `pickscribe` systemd user service.

For AI cleanup, create an Anthropic API key in the [Anthropic console](https://console.anthropic.com/). A dedicated key with a low spend limit is safest. Put it in `~/.config/pickscribe/env`:

```bash
mkdir -p ~/.config/pickscribe
printf 'ANTHROPIC_API_KEY=%s\n' 'sk-ant-...' >> ~/.config/pickscribe/env
chmod 600 ~/.config/pickscribe/env
```

PickScribe reads the key for each dictation, so no restart is needed. Without a key, Right Alt pastes the raw transcript and the capsule says so.

## Settings

Settings live in `~/.config/pickscribe/pickscribe.toml`. Every setting is optional:

```toml
# Languages you speak. PickScribe picks the most likely one for each recording.
languages = ["pt", "en"]

# Names and terms to spell exactly. Both Whisper and the cleanup use this list.
vocabulary = ["Pickforge", "PickScribe"]

# Extra cleanup rules, added to the built-in ones.
cleanup_instructions = "Write numbers as digits."

cleanup_model = "claude-haiku-5-5"
whisper_model = "/home/you/.local/share/pickscribe/models/ggml-large-v3-turbo.bin"
max_recording_secs = 300
```

Restart the service after you change settings: `systemctl --user restart pickscribe`.

## Commands

```bash
pickscribe                         # run the daemon (the service does this)
pickscribe toggle [--raw]          # same as tapping Right Alt (or Shift + Right Alt)
pickscribe cancel                  # same as Esc
pickscribe transcribe file.wav     # transcribe a 16 kHz mono WAV and print the text
pickscribe transcribe --clean f.wav  # same, with AI cleanup
```

`toggle` and `cancel` let you bind other keys in your compositor. Logs go to the journal: `journalctl --user -u pickscribe`.

## Privacy

Audio never leaves your machine. With Right Alt, the transcript text goes to the Anthropic API for cleanup. With Shift + Right Alt, nothing leaves your machine. PickScribe keeps no history. The pasted text stays on the clipboard, so you can paste it again. See [SECURITY.md](SECURITY.md).

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

MIT
