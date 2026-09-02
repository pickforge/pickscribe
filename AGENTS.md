# PickScribe

Local dictation for Linux and Apple silicon macOS. A Rust core (whisper.cpp transcription, LLM cleanup, paste) with CLI binaries, wrapped in a Tauri 2 and Svelte 5 app under `src-tauri/`.

```
bun install
bun run tauri dev                 # bun run dev is Vite alone
bun run check && bun run lint && bun run test && bun run test:coverage
cargo test --workspace --locked --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p pickscribe-app --features pickscribe-app/custom-protocol   # the production build path; nothing else exercises it
```

The root crate and `src-tauri` are one workspace, but `default-members` is the root, so bare cargo commands quietly skip the app. The `run` skill in `.agents/skills/run` explains how to launch an isolated copy for screenshots without touching your real session.

Things you can't guess:

- Only the cleanup step may send text off the machine. Transcription is always local and local-only mode has to stay honest. Widening this means updating the README privacy section.
- Coverage floors (vitest thresholds, `--fail-under-lines`) are ratchets. Don't lower them.
- The version lives in four files: `package.json`, `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`. Release CI fails if the tag doesn't match `tauri.conf.json`. Smoke the exact draft AppImage on a desktop before publishing.
- Settings, API keys and history live in `~/.config/pickscribe` and `~/.local/share/pickscribe`, shared with the CLI binaries, not `~/.pickforge`.
- ESLint here, not oxlint; brand CSS, not Tailwind.
- The legacy `voice-flow` and `pickscribe-gui` wrappers stay so existing keyboard shortcuts keep working.
