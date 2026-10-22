<p align="center">
  <img src="assets/branding/pickscribe-lockup-horizontal.svg" alt="PickScribe" width="560">
</p>

# PickScribe

Dictation for Linux. Tap Right Alt, speak, and tap it again. PickScribe transcribes your speech on your own GPU, cleans the text with Claude, and pastes it into the app you are using.

A small capsule floats at the bottom of the screen while you dictate. It shows the sound waves while it listens and a moving pulse while it thinks. It never takes keyboard focus, and clicks pass through it.

| Shortcut | What it does |
| --- | --- |
| Right Alt | Start or stop dictation. The text goes through AI cleanup. Accent-colored bars. |
| Shift + Right Alt | Start or stop dictation. The raw transcript is pasted and never leaves your machine. Text-colored bars. |
| Esc | Discard the current recording. |

Right Alt only counts when you tap it alone, so AltGr characters and Alt shortcuts keep working.

On Omarchy the capsule takes its colors from the current theme. Elsewhere it uses the Pickforge colors.

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
git clone https://github.com/pickforge/pickscribe
cd pickscribe
scripts/install
```

The script builds PickScribe, downloads the Whisper `large-v3-turbo` model (1.6 GB, checked against its SHA-256), installs `~/.local/bin/pickscribe`, and starts it as the `pickscribe` systemd user service. Run it again after `git pull` to update.

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

# Extra cleanup rules, added to the built-in ones.
cleanup_instructions = "Write numbers as digits."

cleanup_model = "claude-haiku-5-5"
# low, medium, high, xhigh, or max. xhigh and max need cleanup_thinking = true.
cleanup_effort = "low"
cleanup_thinking = false

# Recent dictations to keep. 0 keeps none.
history_limit = 500

# Look for misheard words after this many AI dictations. 0 turns it off.
review_every = 10

whisper_model = "/home/you/.local/share/pickscribe/models/ggml-large-v3-turbo.bin"
max_recording_secs = 300
```

Restart the service after you change settings: `systemctl --user restart pickscribe`.

## Dictionary

`~/.config/pickscribe/dictionary.txt` holds names and terms to spell exactly, one per line. A line like `Bitforge -> Pickforge` is a correction: Whisper heard the left side, you meant the right side. PickScribe gives the terms to Whisper, replaces each correction in the text, and tells the cleanup about both. Changes apply to the next dictation.

```bash
pickscribe dict add Pickforge
pickscribe dict add Pickforge --heard Bitforge
pickscribe dict remove Pickforge --heard Bitforge
```

AI coding agents can keep the dictionary for you. `pickscribe mcp` is an MCP server with tools to list, add, and remove entries. Register it once:

```bash
claude mcp add --scope user pickscribe -- ~/.local/bin/pickscribe mcp
codex mcp add pickscribe -- ~/.local/bin/pickscribe mcp
```

The server asks agents to check with you when a word in your message looks misheard, and to record the correction only after you confirm it.

PickScribe also looks for misheard words on its own. After every 10 AI dictations, it sends them to Claude with the words Whisper was unsure about, and asks which ones look like a misheard name or term. Each finding becomes a notification. Clicking it opens the Omarchy panel, where you add it to the dictionary or ignore it. Nothing changes until you add it, and ignored suggestions never come back. Raw dictations are never reviewed. Without Omarchy, use `pickscribe suggestions`, then `pickscribe suggestions accept ID` or `ignore ID`. `pickscribe review` runs a review now.

## History

PickScribe keeps your recent dictations, so text is not lost when no text field had focus. `pickscribe history` lists them, and `pickscribe history copy ID` puts one back on the clipboard. `pickscribe history delete ID` and `pickscribe history clear` remove them.

## Omarchy

On Omarchy, `scripts/install` also installs a bar widget. Enable it once with `omarchy plugin enable pickforge.pickscribe`. The icon shows whether PickScribe is listening or thinking. Its panel holds the history, the dictionary, the suggested corrections, and the timings of recent dictations.

## Commands

```bash
pickscribe                         # run the daemon (the service does this)
pickscribe toggle [--raw]          # same as tapping Right Alt (or Shift + Right Alt)
pickscribe cancel                  # same as Esc
pickscribe transcribe file.wav     # transcribe a 16 kHz mono WAV and print the text
pickscribe transcribe --clean f.wav  # same, with AI cleanup
pickscribe clean [--effort low] < t.txt  # clean a transcript from stdin
pickscribe status                  # the daemon state as one JSON line
pickscribe watch                   # the same, live, one line per change
```

`toggle` and `cancel` let you bind other keys in your compositor. Logs go to the journal: `journalctl --user -u pickscribe`.

## Privacy

Audio never leaves your machine. With Right Alt, the transcript text and your dictionary go to the Anthropic API for cleanup, and again later for the misheard word review. With Shift + Right Alt, nothing leaves your machine. Recent dictations stay in `~/.local/share/pickscribe/history.jsonl`, readable only by you. Set `history_limit = 0` to keep none. See [SECURITY.md](SECURITY.md).

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

MIT © 2026 Pickforge Studio

---

<p align="center">
  <a href="https://pickforge.dev">
    <img src="assets/branding/pickforge-studio-footer.svg" alt="Pickforge Studio" width="560">
  </a>
</p>
