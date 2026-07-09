---
name: run
description: Launch PickScribe in dev mode or headless lab mode to verify changes without touching the live desktop session when asked to run the app, screenshot it, or confirm a change works in the real app.
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

Use this for autonomous visual verification. A lab identity prevents the single-instance plugin from pinging the user's app, but it does **not** isolate PickScribe's XDG config/data directories.
Never use Clear all, delete history, sign out, or other destructive UI in a lab instance.

Pick an unused display and frontend port, then start Vite separately:

```bash
DISPLAY_NUM=97
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

GDK otherwise prefers Wayland: `DISPLAY=:N` alone can open on the live desktop. The app also has a tray icon, so give it a private D-Bus session. Force X11 and use the lab-only identifier and Vite URL:

```bash
dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 DISPLAY=":$DISPLAY_NUM" \
  bun run tauri dev --config \
  '{"identifier":"com.pickforge.pickscribe.labtest","build":{"devUrl":"http://127.0.0.1:'"$PORT"'","beforeDevCommand":""}}' \
  >"/tmp/pickscribe-lab-$DISPLAY_NUM.log" 2>&1 &
```

Do not pass `--hidden`: the normal config starts the `main` window visible. Wait for the Rust build; use `tail -f "/tmp/pickscribe-lab-$DISPLAY_NUM.log"` if needed.

## Verify and screenshot

Inspect the visible window, then capture the Xvfb root. ImageMagick `import` is available here:

```bash
import -display ":$DISPLAY_NUM" -window root "/tmp/pickscribe-lab-$DISPLAY_NUM.png"
```

## Cleanup

Find each PID first with `pgrep -f` (bracketed patterns avoid matching the search). Confirm it is the lab, then
run the matching `bash` kill command separately:

```bash
pgrep -af "$PWD/node_modules/.bin/[t]auri dev --config"
bash -lc 'kill <tauri-cli-pid>'
pgrep -af "$PWD/target/debug/[p]ickscribe-app"
bash -lc 'kill <lab-app-pid>'
pgrep -af "[v]ite.*--port $PORT"
bash -lc 'kill <vite-pid>'
pgrep -af "[x]fwm4 --display=:$DISPLAY_NUM"
bash -lc 'kill <xfwm4-pid>'
pgrep -af "[X]vfb :$DISPLAY_NUM"
bash -lc 'kill <xvfb-pid>'
```

Each `bash -lc` call must do nothing except kill the inspected PID. Never use `pkill -f` in a compound command:
its pattern can match the wrapper shell and terminate it instead (exit 144).
