---
name: run
description: Launch PickScribe in dev mode or a sandboxed headless lab profile when asked to run the app, screenshot it, or confirm a change works in the real app.
---

## Normal dev launch

Use only in an empty desktop session you own. The real identifier is `com.pickforge.pickscribe`; a plain launch contacts any running instance and focuses its window.
```bash
bun install --frozen-lockfile
bun run tauri dev
```
There is no codegen or sidecar build. It needs Bun, Rust, GTK3, WebKit2GTK 4.1, libayatana-appindicator, librsvg, OpenSSL; audio also needs whisper.cpp and system tools.

## Isolated lab launch

Each lab starts from an empty, safe profile; anything outside HOME/XDG isolation (inherited environment variables, portals, and system services) can still touch real state.
One lab per checkout: state lives in `$PWD/.lab.env`, so concurrent labs need separate worktrees. Prepare the profile:
```bash
set -e
[ ! -f "$PWD/.lab.env" ] || { echo "lab state exists — clean up the previous lab first" >&2; false; }
bun install --frozen-lockfile
command -v ss >/dev/null || { echo "ss (iproute2) required" >&2; false; }
REAL_HOME=$HOME; LAB_HOME=$(mktemp -d /tmp/pickscribe-lab-home.XXXX)
mkdir -m 700 "$LAB_HOME/runtime"
for n in $(seq 90 120); do [ ! -e "/tmp/.X11-unix/X$n" ] && DISPLAY_NUM=$n && break; done
: "${DISPLAY_NUM:?No free X display in 90-120}"
for candidate in {1421..1439}; do ! ss -ltnH "sport = :$candidate" | grep -q . && { PORT=$candidate; break; }; done
: "${PORT:?No free port in 1421-1439}"
mkdir -p "$LAB_HOME/.config/pickscribe"
printf '%s\n' '[general]' 'local_only = true' '' '[paste]' 'method = "none"' 'copy_to_clipboard = false' > "$LAB_HOME/.config/pickscribe/config.toml"
printf 'REAL_HOME=%s\nPORT=%s\nDISPLAY_NUM=%s\nLAB_HOME=%s\n' "$REAL_HOME" "$PORT" "$DISPLAY_NUM" "$LAB_HOME" > "$PWD/.lab.env"
```
Start Vite, Xvfb, and the lab. Wait for Xvfb before starting xfwm4 or the app:
```bash
source "$PWD/.lab.env"
set -e
setsid bun run dev -- --port "$PORT" --strictPort </dev/null >"/tmp/pickscribe-vite-$PORT.log" 2>&1 &
for _ in {1..60}; do curl -fsS "http://127.0.0.1:$PORT/" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "http://127.0.0.1:$PORT/" >/dev/null || { echo "Vite never became ready on :$PORT" >&2; false; }
setsid Xvfb ":$DISPLAY_NUM" -screen 0 1440x1000x24 -nolisten tcp </dev/null >"/tmp/pickscribe-xvfb-$DISPLAY_NUM.log" 2>&1 &
for _ in {1..50}; do [ -e "/tmp/.X11-unix/X$DISPLAY_NUM" ] && break; sleep 0.1; done
[ -e "/tmp/.X11-unix/X$DISPLAY_NUM" ] || { echo "Xvfb did not start" >&2; false; }
setsid xfwm4 --display=":$DISPLAY_NUM" --compositor=off </dev/null >"/tmp/pickscribe-xfwm-$DISPLAY_NUM.log" 2>&1 &
setsid dbus-run-session -- env -u WAYLAND_DISPLAY -u DEEPSEEK_API_KEY -u OPENAI_API_KEY -u OLLAMA_API_KEY -u PICKSCRIBE_API_KEY GDK_BACKEND=x11 DISPLAY=":$DISPLAY_NUM" \
  HOME="$LAB_HOME" XDG_CONFIG_HOME="$LAB_HOME/.config" XDG_DATA_HOME="$LAB_HOME/.local/share" XDG_CACHE_HOME="$LAB_HOME/.cache" \
  XDG_RUNTIME_DIR="$LAB_HOME/runtime" PICKSCRIBE_STATE_DIR="$LAB_HOME/state" CARGO_HOME="$REAL_HOME/.cargo" RUSTUP_HOME="$REAL_HOME/.rustup" \
  bun run tauri dev --config '{"identifier":"com.pickforge.pickscribe.labtest'"$DISPLAY_NUM"'","build":{"devUrl":"http://127.0.0.1:'"$PORT"'","beforeDevCommand":""}}' </dev/null >"/tmp/pickscribe-lab-$DISPLAY_NUM.log" 2>&1 &
```
GDK otherwise prefers Wayland: `DISPLAY=:N` alone can open on the live desktop. The private D-Bus session isolates the tray; `REAL_HOME` preserves Cargo and Rustup. The identifier suffix (`.labtest$DISPLAY_NUM`) avoids the live app AND other agents' concurrent labs — two labs on one identifier ping each other and the second exits. The seeded config disables cloud cleanup and typing; `method = "none"` still copies to a clipboard, but the private `XDG_RUNTIME_DIR` keeps `wl-copy` (default-socket lookup) and `ydotool` off the real session — clipboard writes land only on the lab display. Never test dictation paste-through against the real session.

Basic boot needs no real-profile files. To test dictation, opt in to model and whisper-cli access before launch:
```bash
source "$PWD/.lab.env"
WHISPER_CLI=$(command -v whisper-cli || echo "$REAL_HOME/.local/bin/whisper-cli")
[ -x "$WHISPER_CLI" ] || { echo "whisper-cli required" >&2; false; }
mkdir -p "$LAB_HOME/.local/bin" "$LAB_HOME/.local/share/whisper.cpp"
ln -s "$WHISPER_CLI" "$LAB_HOME/.local/bin/whisper-cli"
ln -s "$REAL_HOME/.local/share/whisper.cpp/models" "$LAB_HOME/.local/share/whisper.cpp/models"
```
Do not link real config, data, or credentials. Do not pass `--hidden`: the normal config starts the `main` window visible.

## Verify and screenshot

Wait for the Rust build, inspect the visible window, then capture the Xvfb root. ImageMagick `import` is available here:
```bash
source "$PWD/.lab.env"
import -display ":$DISPLAY_NUM" -window root "/tmp/pickscribe-lab-$DISPLAY_NUM.png"
find "$LAB_HOME" -maxdepth 5 -type f
```

## Cleanup

Find each PID first with `pgrep -f` (bracketed patterns avoid matching the search). Confirm the app PID has `$LAB_HOME` in its environment before killing it:
```bash
[ -f "$PWD/.lab.env" ] || { echo "no lab state in this checkout" >&2; false; }
source "$PWD/.lab.env"
: "${PORT:?}" "${DISPLAY_NUM:?}" "${LAB_HOME:?}"
pgrep -af "$PWD/target/debug/[p]ickscribe-app"
tr '\0' '\n' < /proc/<lab-app-pid>/environ | grep -qF "$LAB_HOME" || { echo "not the lab app" >&2; false; }
bash -lc 'kill <lab-app-pid>'
pgrep -af "$PWD/node_modules/.bin/[t]auri dev --config.*com.pickforge.pickscribe.labtest$DISPLAY_NUM"
bash -lc 'kill <tauri-cli-pid>'
pgrep -af "[v]ite.*--port $PORT"
bash -lc 'kill <vite-pid>'
pgrep -af "[x]fwm4 --display=:$DISPLAY_NUM"
bash -lc 'kill <xfwm4-pid>'
pgrep -af "[X]vfb :$DISPLAY_NUM"
bash -lc 'kill <xvfb-pid>'
rm -rf "$LAB_HOME"; rm -f "$PWD/.lab.env"
```
Each `bash -lc` call must do nothing except kill the inspected PID. Never use `pkill -f` in a compound command: its pattern can match the wrapper shell and terminate it instead (exit 144).
