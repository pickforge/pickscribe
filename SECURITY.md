# Security policy

PickScribe is a Linux dictation daemon. It records your microphone while you dictate, transcribes the audio locally with Whisper, optionally cleans the transcript with Claude, and pastes the result into the focused app.

## What leaves the machine, and when

| Data | Leaves the machine? | When |
| --- | --- | --- |
| Microphone audio | Never | Transcribed locally. The microphone is open only while a recording runs. |
| Transcript text | Only after Right Alt (cleanup mode) | Sent to the Anthropic Messages API for cleanup. Shift + Right Alt at either tap never sends it. |
| Dictionary | Only after Right Alt (cleanup mode) | Its terms and corrections go with the transcript, to guide the cleanup. |
| AI dictations, again | Every `review_every` AI dictations | Sent once more, with the words Whisper was unsure about, to look for misheard words. Raw dictations never are. Set `review_every = 0` to stop it. |
| History | Never | The last `history_limit` dictations, in `~/.local/share/pickscribe/history.jsonl` with mode 600. |
| API key | Only to the Anthropic API | Read from `~/.config/pickscribe/env` or `ANTHROPIC_API_KEY`. Never logged. |
| Settings | Never | `~/.config/pickscribe/pickscribe.toml` |

AI mode opens the HTTPS connection to Anthropic when recording starts, before any text exists. This connection sends no transcript or API key.

PickScribe sends no telemetry or crash reports. Logs contain errors and timings only, never transcripts. History keeps text, never audio; set `history_limit = 0` to keep none.

## Local access

- Right Alt is read from `/dev/input`, which needs membership in the `input` group. That group can read every keystroke, so PickScribe tracks held keys to detect Right Alt taps. It stores no keystrokes.
- The pasted text stays on the Wayland clipboard until something else replaces it. Clipboard managers, such as Omarchy's clipboard history, keep their own copy.
- The control socket lives in `$XDG_RUNTIME_DIR`, which only your user can open. Its status stream holds state and timings, never text.
- `pickscribe mcp` lets AI agents you register read and edit the dictionary. It cannot read history or record audio.
- Keep `~/.config/pickscribe/env` at mode `600`. Use a dedicated Anthropic key with a spend limit.

## Android app

The Android app in `mobile/` records and transcribes on the phone. Only model downloads from Hugging Face leave the phone, and they carry no audio or text. History, the dictionary, and settings stay in the app's private storage, and Android backups exclude them.

- To insert text, the app puts it on the clipboard, pastes it, and clears the clipboard a second later. The clip is marked sensitive, but Android does not enforce that mark. A keyboard's clipboard history or a cross-device clipboard sync, such as Link to Windows, may keep a copy.
- When the text cannot be inserted, it stays on the clipboard so you can paste it yourself.
- Logs hold app names, reasons, and timings, never text.

## Reporting a vulnerability

Please report security issues privately:

- GitHub security advisories: <https://github.com/pickforge/pickscribe/security/advisories/new>
- Email: <security@pickforge.dev>

Do not open public issues for security reports. We aim to acknowledge within a few days.
