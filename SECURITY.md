# Security policy

PickScribe is a Linux dictation daemon. It records your microphone while you dictate, transcribes the audio locally with Whisper, optionally cleans the transcript with Claude, and pastes the result into the focused app.

## What leaves the machine, and when

| Data | Leaves the machine? | When |
| --- | --- | --- |
| Microphone audio | Never | Transcribed locally. The microphone is open only while a recording runs. |
| Transcript text | Only after Right Alt (cleanup mode) | Sent to the Anthropic Messages API for cleanup. Shift + Right Alt never sends it. |
| API key | Only to the Anthropic API | Read from `~/.config/pickscribe/env` or `ANTHROPIC_API_KEY`. Never logged. |
| Settings | Never | `~/.config/pickscribe/pickscribe.toml` |

PickScribe keeps no history and sends no telemetry or crash reports. Logs contain errors only, never transcripts.

## Local access

- Right Alt is read from `/dev/input`, which needs membership in the `input` group. That group can read every keystroke, so PickScribe only reacts to Right Alt, Shift, and Esc, and stores nothing.
- The pasted text stays on the Wayland clipboard until something else replaces it.
- The control socket lives in `$XDG_RUNTIME_DIR`, which only your user can open.
- Keep `~/.config/pickscribe/env` at mode `600`. Use a dedicated Anthropic key with a spend limit.

## Reporting a vulnerability

Please report security issues privately:

- GitHub security advisories: <https://github.com/pickforge/pickscribe/security/advisories/new>
- Email: <security@pickforge.dev>

Do not open public issues for security reports. We aim to acknowledge within a few days.
