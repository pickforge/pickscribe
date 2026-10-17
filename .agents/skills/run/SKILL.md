---
name: run
description: Launch the PickScribe daemon from this checkout and check the capsule, transcription, or cleanup without pasting into the user's session.
---

Stopping a recording pastes into the focused window. Never stop one with `toggle` while the user works; end it with `cancel` instead.

A second daemon would clash with the installed service on the control socket. Run the checkout build against its own runtime directory, keeping the real Wayland socket:

```bash
cargo build --release
lab=$(mktemp -d)
export WAYLAND_DISPLAY="$XDG_RUNTIME_DIR/${WAYLAND_DISPLAY:-wayland-1}" XDG_RUNTIME_DIR="$lab"
./target/release/pickscribe &
./target/release/pickscribe toggle   # capsule appears and listens
hyprctl layers -j                    # look for the "pickscribe" namespace
./target/release/pickscribe cancel   # capsule hides, microphone closes
kill %1; rm -rf "$lab"
```

Both daemons react to a real Right Alt tap, so do not tap it during a check.

Check transcription and cleanup on a file instead of the microphone:

```bash
./target/release/pickscribe transcribe sample.wav           # 16 kHz mono WAV
./target/release/pickscribe transcribe --clean sample.wav   # needs ANTHROPIC_API_KEY
```
