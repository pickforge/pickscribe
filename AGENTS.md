The root crate and `src-tauri` share a Cargo workspace, but `default-members` selects only the root. Use workspace-wide checks for changes crossing that boundary. Production app integration changes also need `cargo check -p pickscribe-app --features pickscribe-app/custom-protocol`; ordinary development checks do not exercise that feature.

`bun run tauri dev` launches the app; `bun run dev` is Vite alone. `.agents/skills/run` describes an isolated launch that does not touch the real session.

Only cleanup may send text off the machine. Transcription stays local, and local-only mode must remain honest. Expanding that boundary requires updating the README privacy section. Do not lower vitest or Rust coverage floors.

Settings, keys and history live in `~/.config/pickscribe` and `~/.local/share/pickscribe`, shared with the CLI binaries, not `~/.pickforge`. Keep the legacy `voice-flow` and `pickscribe-gui` wrappers working for existing keyboard shortcuts.

This app uses ESLint and brand CSS, not the workspace oxlint/Tailwind defaults. Release CI checks the tag against `src-tauri/tauri.conf.json`; smoke the exact draft AppImage on a desktop before human publication.
