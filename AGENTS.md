PickScribe is a single Rust binary: a daemon with a GTK4 layer-shell overlay, plus `toggle`, `cancel`, and `transcribe` subcommands. Check with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`. The first build compiles whisper.cpp with Vulkan, which needs `cmake`, `clang`, `glslc`, and Vulkan headers.

Only cleanup may send text off the machine, and only the transcript. Audio stays local, Shift + Right Alt must never reach the network, and logs must never contain transcripts. Expanding that boundary requires updating the privacy sections of README.md and SECURITY.md.

Settings and keys live in `~/.config/pickscribe`, models in `~/.local/share/pickscribe/models`. The legacy Tauri app also stored `config.toml`, `env`, and `pickscribe.db` there; never rewrite or delete them. The new settings file is `pickscribe.toml`.

`.agents/skills/run` describes how to launch and check the daemon without pasting into the user's session.
