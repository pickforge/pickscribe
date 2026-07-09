---
name: run
description: Launch PickScribe in dev mode or a sandboxed headless lab profile when asked to run the app, screenshot it, or confirm a change works in the real app.
---

# Run PickScribe

## Normal dev launch

Use only in an empty desktop session you own. The real identifier is `com.pickforge.pickscribe`; a plain launch contacts any running instance and focuses its window.
```bash
bun install --frozen-lockfile
bun run tauri dev
```
There is no codegen or sidecar build. It needs Bun, Rust, GTK3, WebKit2GTK 4.1, libayatana-appindicator, librsvg, OpenSSL; audio also needs whisper.cpp and system tools.

## Isolated lab launch

Each lab starts from an empty profile; destructive UI changes only `$LAB_HOME`. Inherited environment variables, portals, and system services are outside HOME/XDG isolation.
Pick unused X and Vite ports, then start Vite separately:
```bash
REAL_HOME=$HOME; LAB_HOME=$(mktemp -d /tmp/pickscribe-lab-home.XXXX)
for n in $(seq 90 120); do [ ! -e "/tmp/.X11-unix/X$n" ] && DISPLAY_NUM=$n && break; done
: "${DISPLAY_NUM:?No free X display in 90-120}"
for candidate in {1421..1439}; do ! ss -ltnH "sport = :$candidate" | grep -q . && { PORT=$candidate; break; }; done
: "${PORT:?No free port in 1421-1439}"

bun run dev -- --port "$PORT" --strictPort >"/tmp/pickscribe-vite-$PORT.log" 2>&1 &
for _ in {1..60}; do curl -fsS "http://127.0.0.1:$PORT/" >/dev/null && break; sleep 1; done
curl -fsS "http://127.0.0.1:$PORT/" >/dev/null
```
Start an Xvfb display and a window manager. `xfwm4` is required for maximize/restore controls:
```bash
Xvfb ":$DISPLAY_NUM" -screen 0 1440x1000x24 -nolisten tcp >"/tmp/pickscribe-xvfb-$DISPLAY_NUM.log" 2>&1 &
xfwm4 --display=":$DISPLAY_NUM" --compositor=off >"/tmp/pickscribe-xfwm-$DISPLAY_NUM.log" 2>&1 &
```
GDK otherwise prefers Wayland: `DISPLAY=:N` alone can open on the live desktop. The app has a tray icon, so give it a private D-Bus session. `REAL_HOME` preserves the existing Cargo and Rustup installations:
```bash
dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 DISPLAY=":$DISPLAY_NUM" \
  HOME="$LAB_HOME" XDG_CONFIG_HOME="$LAB_HOME/.config" XDG_DATA_HOME="$LAB_HOME/.local/share" XDG_CACHE_HOME="$LAB_HOME/.cache" \
  CARGO_HOME="$REAL_HOME/.cargo" RUSTUP_HOME="$REAL_HOME/.rustup" \
  bun run tauri dev --config \
  '{"identifier":"com.pickforge.pickscribe.labtest","build":{"devUrl":"http://127.0.0.1:'"$PORT"'","beforeDevCommand":""}}' \
  >"/tmp/pickscribe-lab-$DISPLAY_NUM.log" 2>&1 &
```
Basic boot needs no real-profile files. To test dictation, opt in to model-only access before launch:
```bash
mkdir -p "$LAB_HOME/.local/share/whisper.cpp"
ln -s "$REAL_HOME/.local/share/whisper.cpp/models" "$LAB_HOME/.local/share/whisper.cpp/models"
```
Do not link real config, data, or credentials. Do not pass `--hidden`: the normal config starts the `main` window visible.

## Verify and screenshot

Wait for the Rust build, inspect the visible window, then capture the Xvfb root. ImageMagick `import` is available here:
```bash
import -display ":$DISPLAY_NUM" -window root "/tmp/pickscribe-lab-$DISPLAY_NUM.png"
find "$LAB_HOME" -maxdepth 5 -type f
```

## Cleanup

Find each PID first with `pgrep -f` (bracketed patterns avoid matching the search). Confirm it is the lab, then run the matching `bash` kill command separately:
```bash
pgrep -af "$PWD/target/debug/[p]ickscribe-app"
bash -lc 'kill <lab-app-pid>'
pgrep -af "$PWD/node_modules/.bin/[t]auri dev --config"
bash -lc 'kill <tauri-cli-pid>'
pgrep -af "[v]ite.*--port $PORT"
bash -lc 'kill <vite-pid>'
pgrep -af "[x]fwm4 --display=:$DISPLAY_NUM"
bash -lc 'kill <xfwm4-pid>'
pgrep -af "[X]vfb :$DISPLAY_NUM"
bash -lc 'kill <xvfb-pid>'
rm -rf "$LAB_HOME"
```
Each `bash -lc` call must do nothing except kill the inspected PID. Never use `pkill -f` in a compound command: its pattern can match the wrapper shell and terminate it instead (exit 144).
