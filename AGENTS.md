The desktop PickScribe is a single Rust binary: a daemon with a GTK4 layer-shell overlay, plus subcommands such as `toggle`, `cancel`, and `transcribe`. Check with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`. The first build compiles whisper.cpp with Vulkan, which needs `cmake`, `clang`, `glslc`, and Vulkan headers.

Only cleanup and the misheard word review may send text off the machine, and only AI-mode transcripts and the dictionary. Audio stays local and is never saved, Shift + Right Alt must never reach the network, and logs and the status stream must never contain transcripts. History is the only place text is stored. On Android, only model downloads may reach the network. Expanding that boundary requires updating the privacy sections of README.md and SECURITY.md.

Settings and keys live in `~/.config/pickscribe`, models in `~/.local/share/pickscribe/models`. The legacy Tauri app also stored `config.toml`, `env`, and `pickscribe.db` there; never rewrite or delete them. The new settings file is `pickscribe.toml`.

The Android app in `mobile/` is Flutter, with a Kotlin accessibility service for the bubble and whisper.cpp built through the NDK. In `mobile/`, check with `fvm dart format --output=none --set-exit-if-changed .`, `fvm flutter analyze`, and `fvm flutter test`. CI does not run these yet.

`.agents/skills/run` describes how to launch and check the daemon without pasting into the user's session. `scripts/omarchy-lab` runs an invisible Omarchy shell for checking the bar widget; never open panels or take screenshots on the user's real display.
