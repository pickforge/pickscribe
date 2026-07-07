# Security policy

PickScribe is a local-first Linux dictation app. It records your microphone, transcribes the audio locally with `whisper.cpp`, optionally cleans the transcript with a configured LLM, and pastes the result into the focused app. It is built so your voice stays on your machine.

## Privacy and security model

- **Audio never leaves your machine.** Recordings are transcribed locally with the bundled `whisper.cpp` flow and are never uploaded.
- **Cleanup sends text, never audio.** When LLM cleanup is enabled, only the transcribed text is sent to the configured LLM endpoint — DeepSeek by default — and only for cleanup.
- **Incremental mode stays local by default.** Opt-in incremental dictation may write partial transcript JSON under the local runtime state directory while recording. Those files are removed on stop/cancel unless audio retention is enabled.
- **Segment cleanup is a separate opt-in.** If enabled, finalized partial transcript text may be sent to the configured cleanup provider before recording stops. Audio still never leaves the machine.
- **Local-only mode.** One switch restricts cleanup to loopback endpoints (Ollama, LM Studio, llama.cpp server…), blocks remote providers, and falls back to the raw transcript — so no text leaves the machine either.
- **No telemetry.** PickScribe makes no analytics or telemetry calls. Outbound requests are limited to the optional cleanup call described above and the startup update check below.
- **Update check.** Packaged builds check GitHub Releases for a newer version on startup (they fetch the release `latest.json` — version metadata only, never your transcripts, audio, or documents). This runs even in Local-only mode and with cleanup disabled.
- **Secrets on disk.** API keys live in `~/.config/pickscribe/env`, which should be `chmod 600`. PickScribe never intentionally prints API keys; docs and diagnostics redact secrets.

## What leaves the machine, and when

| Data | Leaves the machine? | When |
| --- | --- | --- |
| Microphone audio | Never | Transcribed locally with `whisper.cpp` |
| Incremental partial transcript text | Only with segment cleanup enabled and Local-only mode off | Stored temporarily in the runtime state directory while opt-in incremental mode is active; finalized partial text may be sent for cleanup only when explicitly enabled |
| Transcribed text | Only with LLM cleanup enabled and Local-only mode off | Sent to the configured endpoint (DeepSeek by default) for cleanup |
| API keys, history, settings | Never | Stored locally under `~/.config/pickscribe` and `~/.local/share/pickscribe` |
| App update check | Yes — a request to GitHub | On startup (packaged builds); fetches the release `latest.json` to compare versions. No document or audio content is sent. |

## Reporting a vulnerability

Please report security issues privately:

- GitHub security advisories: <https://github.com/pickforge/pickscribe/security/advisories/new>
- Email: <security@pickforge.dev>

Do not open public issues for security reports. We aim to acknowledge within a few days.
