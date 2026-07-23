mod engine;
mod file_job;
mod kwin;
mod lifecycle;
mod settings;
mod tray;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pickscribe::config::AppConfig;
use pickscribe::engine::{cleanup, command_exists, media, stt, transcript};
use pickscribe::history::{HistoryEntry, Metrics};
use pickscribe::platform::{self, PlatformSupport};
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_dialog::DialogExt;

use engine::{Engine, StatePayload};

type CommandResult<T> = Result<T, String>;

const SENTRY_DSN: &str = "https://241506ecc655d5fdb4c68b69f8b9c548@o4511699702317056.ingest.us.sentry.io/4511699813859328";
fn basename(value: &str) -> String {
    let trimmed = value.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(value)
        .to_string()
}

fn strip_debug_image_paths(event: &mut sentry::protocol::Event<'_>) {
    for image in &mut event.debug_meta.to_mut().images {
        match image {
            sentry::protocol::DebugImage::Apple(image) => {
                image.name = basename(&image.name);
            }
            sentry::protocol::DebugImage::Symbolic(image) => {
                image.name = basename(&image.name);
                if let Some(debug_file) = &mut image.debug_file {
                    *debug_file = basename(debug_file);
                }
            }
            sentry::protocol::DebugImage::Wasm(image) => {
                image.code_file = basename(&image.code_file);
                if let Some(debug_file) = &mut image.debug_file {
                    *debug_file = basename(debug_file);
                }
            }
            _ => {}
        }
    }
}

fn err_string(err: impl std::fmt::Display) -> String {
    format!("{err}")
}

#[tauri::command]
fn get_state(engine: State<'_, Arc<Engine>>) -> StatePayload {
    engine.state()
}

#[tauri::command]
fn toggle_dictation(app: AppHandle, engine: State<'_, Arc<Engine>>) {
    engine.set_chord_override(None);
    engine.toggle(&app);
}

/// Parse `--paste-chord=ctrl-shift-v` style args from a CLI invocation.
fn parse_chord_arg(args: &[String]) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix("--paste-chord="))
        .filter(|v| matches!(*v, "ctrl-v" | "ctrl-shift-v"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parse_chord_arg_accepts_supported_paste_chords() {
        assert_eq!(
            parse_chord_arg(&args(&["pickscribe-app", "--paste-chord=ctrl-v"])).as_deref(),
            Some("ctrl-v")
        );
        assert_eq!(
            parse_chord_arg(&args(&["pickscribe-app", "--paste-chord=ctrl-shift-v"])).as_deref(),
            Some("ctrl-shift-v")
        );
    }

    #[test]
    fn parse_chord_arg_rejects_missing_or_unsupported_chords() {
        assert_eq!(parse_chord_arg(&args(&["pickscribe-app", "--toggle"])), None);
        assert_eq!(
            parse_chord_arg(&args(&["pickscribe-app", "--paste-chord=ctrl-alt-v"])),
            None
        );
    }

    #[test]
    fn strip_debug_image_paths_basenames_all_image_variants() {
        let apple_uuid = "2df005a8-67ab-4d33-98f2-52f9f6de4d15";
        let symbolic_id = "494f3aea-88fa-4296-9644-fa8ef5d139b6-1234";
        let wasm_id = "8c954262-f905-4992-8a61-f60825f4553b";
        let mut event = sentry::protocol::Event {
            debug_meta: std::borrow::Cow::Owned(sentry::protocol::DebugMeta {
                images: vec![
                    sentry::protocol::AppleDebugImage {
                        name: "/Users/alice/Applications/PickScribe.app/Contents/MacOS/PickScribe"
                            .into(),
                        arch: Some("arm64".into()),
                        cpu_type: Some(16_777_228),
                        cpu_subtype: Some(0),
                        image_addr: 4096.into(),
                        image_size: 8192,
                        image_vmaddr: 12288.into(),
                        uuid: apple_uuid.parse().unwrap(),
                    }
                    .into(),
                    sentry::protocol::SymbolicDebugImage {
                        name: "/home/alice/Applications/PickScribe.AppImage".into(),
                        arch: Some("x86_64".into()),
                        image_addr: 0.into(),
                        image_size: 4096,
                        image_vmaddr: 0.into(),
                        id: symbolic_id.parse().unwrap(),
                        code_id: None,
                        debug_file: Some("C:\\Users\\alice\\pickscribe.debug".into()),
                    }
                    .into(),
                    sentry::protocol::WasmDebugImage {
                        name: "pickscribe_bg.wasm".into(),
                        debug_id: wasm_id.parse().unwrap(),
                        debug_file: Some("/home/alice/debug/pickscribe_bg.wasm.debug".into()),
                        code_id: Some("abc123".into()),
                        code_file: "C:\\Users\\alice\\pickscribe_bg.wasm".into(),
                    }
                    .into(),
                ],
                ..Default::default()
            }),
            ..Default::default()
        };

        strip_debug_image_paths(&mut event);

        match &event.debug_meta.images[0] {
            sentry::protocol::DebugImage::Apple(image) => {
                assert_eq!(image.name, "PickScribe");
                assert_eq!(image.uuid.to_string(), apple_uuid);
                assert_eq!(image.arch.as_deref(), Some("arm64"));
            }
            other => panic!("expected apple debug image, got {other:?}"),
        }
        match &event.debug_meta.images[1] {
            sentry::protocol::DebugImage::Symbolic(image) => {
                assert_eq!(image.name, "PickScribe.AppImage");
                assert_eq!(image.debug_file.as_deref(), Some("pickscribe.debug"));
                assert_eq!(image.id.to_string(), symbolic_id);
            }
            other => panic!("expected symbolic debug image, got {other:?}"),
        }
        match &event.debug_meta.images[2] {
            sentry::protocol::DebugImage::Wasm(image) => {
                assert_eq!(image.code_file, "pickscribe_bg.wasm");
                assert_eq!(image.debug_file.as_deref(), Some("pickscribe_bg.wasm.debug"));
                assert_eq!(image.debug_id.to_string(), wasm_id);
            }
            other => panic!("expected wasm debug image, got {other:?}"),
        }
    }

    fn history_entry() -> HistoryEntry {
        HistoryEntry {
            id: 42,
            created_at: 0,
            duration_ms: 1_000,
            raw_text: "raw transcript".into(),
            cleaned_text: Some("clean transcript".into()),
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: Some("/home/me/meeting.mp4".into()),
            segments_json: Some(
                r#"[{"start_ms":0,"end_ms":1000,"text":"First sentence."}]"#.into(),
            ),
            word_count: 2,
        }
    }

    #[test]
    fn export_content_uses_cleaned_text_and_timestamped_segments() {
        let entry = history_entry();

        assert_eq!(export_content(&entry, "txt").unwrap(), "clean transcript");
        assert_eq!(
            export_content(&entry, "srt").unwrap(),
            "1\n00:00:00,000 --> 00:00:01,000\nFirst sentence.\n"
        );
        assert_eq!(export_file_name(&entry, "vtt"), "meeting.vtt");
    }

    #[test]
    fn timestamped_exports_require_segments() {
        let mut entry = history_entry();
        entry.segments_json = None;

        assert_eq!(
            export_content(&entry, "vtt").unwrap_err(),
            "no timestamped segments for this entry"
        );
        assert_eq!(
            export_file_name(
                &HistoryEntry {
                    source_file: None,
                    ..entry
                },
                "txt"
            ),
            "transcript-42.txt"
        );
    }
}

#[tauri::command]
fn cancel_dictation(app: AppHandle, engine: State<'_, Arc<Engine>>) {
    engine.cancel(&app);
}

#[tauri::command]
fn get_app_config() -> AppConfig {
    AppConfig::load()
}

#[tauri::command]
fn get_platform_support() -> PlatformSupport {
    platform::current()
}

#[tauri::command]
fn update_app_config(app: AppHandle, config: AppConfig) -> CommandResult<AppConfig> {
    settings::replace(&app, config)
}

#[tauri::command]
async fn get_system_theme(app: AppHandle, engine: State<'_, Arc<Engine>>) -> CommandResult<String> {
    let stage = engine.state().stage;
    tauri::async_runtime::spawn_blocking(move || {
        // The frontend calls this on explicit theme events (media-query
        // change, window focus), so force a fresh probe and re-sync the
        // tray icon in case the panel theme flipped.
        let dark = tray::refresh_panel_prefers_dark();
        tray::sync(&app, stage);
        if dark { "dark".into() } else { "light".into() }
    })
    .await
    .map_err(err_string)
}

#[tauri::command]
fn list_history(
    engine: State<'_, Arc<Engine>>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> CommandResult<Vec<HistoryEntry>> {
    engine
        .history
        .list(
            search.as_deref().unwrap_or(""),
            limit.unwrap_or(50).clamp(1, 500),
            offset.unwrap_or(0).max(0),
        )
        .map_err(err_string)
}

#[tauri::command]
fn delete_history_entry(engine: State<'_, Arc<Engine>>, id: i64) -> CommandResult<()> {
    engine.history.delete(id).map_err(err_string)
}

#[tauri::command]
fn clear_history(engine: State<'_, Arc<Engine>>) -> CommandResult<()> {
    engine.history.clear().map_err(err_string)
}

#[tauri::command]
fn transcribe_media_file(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    path: String,
    cleanup: bool,
) -> CommandResult<()> {
    file_job::start(Arc::clone(engine.inner()), app, path, cleanup).map_err(err_string)
}

#[tauri::command]
fn cancel_file_transcription(engine: State<'_, Arc<Engine>>) -> CommandResult<()> {
    file_job::cancel(engine.inner());
    Ok(())
}

#[tauri::command]
async fn pick_media_file(app: AppHandle) -> CommandResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Media", media::MEDIA_EXTENSIONS)
            .blocking_pick_file()
            .map(|path| {
                path.into_path()
                    .map(|path| path.display().to_string())
                    .map_err(err_string)
            })
            .transpose()
    })
    .await
    .map_err(err_string)?
}

#[tauri::command]
async fn export_history_entry(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    id: i64,
    format: String,
) -> CommandResult<Option<String>> {
    let entry = engine
        .history
        .get(id)
        .map_err(err_string)?
        .ok_or_else(|| format!("history entry not found: {id}"))?;
    let content = export_content(&entry, &format)?;
    let file_name = export_file_name(&entry, &format);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .add_filter(format.to_ascii_uppercase(), &[format.as_str()])
            .set_file_name(file_name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(err_string)?;
        fs::write(&path, content).map_err(err_string)?;
        Ok(Some(path.display().to_string()))
    })
    .await
    .map_err(err_string)?
}

fn export_content(entry: &HistoryEntry, format: &str) -> CommandResult<String> {
    match format {
        "txt" => Ok(entry
            .cleaned_text
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(&entry.raw_text)
            .to_string()),
        "srt" | "vtt" => {
            let segments_json = entry
                .segments_json
                .as_deref()
                .ok_or_else(|| "no timestamped segments for this entry".to_string())?;
            let segments: Vec<transcript::FileSegment> =
                serde_json::from_str(segments_json).map_err(err_string)?;
            Ok(if format == "srt" {
                transcript::to_srt(&segments)
            } else {
                transcript::to_vtt(&segments)
            })
        }
        _ => Err(format!("unsupported export format: {format}")),
    }
}

fn export_file_name(entry: &HistoryEntry, extension: &str) -> String {
    let source_name = entry.source_file.as_deref().and_then(|source| {
        Path::new(source)
            .file_stem()
            .and_then(|name| name.to_str())
    });
    match source_name.filter(|name| !name.is_empty()) {
        Some(name) => format!("{name}.{extension}"),
        None => format!("transcript-{}.{}", entry.id, extension),
    }
}

#[tauri::command]
fn get_metrics(engine: State<'_, Arc<Engine>>) -> CommandResult<Metrics> {
    let cfg = AppConfig::load();
    engine
        .history
        .metrics(cfg.general.typing_wpm)
        .map_err(err_string)
}

#[derive(Serialize)]
struct DoctorCheck {
    name: String,
    ok: bool,
    detail: String,
}

#[tauri::command]
async fn run_doctor() -> CommandResult<Vec<DoctorCheck>> {
    tauri::async_runtime::spawn_blocking(run_doctor_checks)
        .await
        .map_err(err_string)
}

fn run_doctor_checks() -> Vec<DoctorCheck> {
    let cfg = AppConfig::load();
    let support = platform::current();
    let mut checks = Vec::new();
    let mut push = |name: &str, ok: bool, detail: String| {
        checks.push(DoctorCheck {
            name: name.into(),
            ok,
            detail,
        });
    };

    push(
        "Release platform",
        support.dictation_supported,
        support.summary.clone(),
    );
    match media::resolve_ffmpeg() {
        Ok(path) => push("ffmpeg", true, path.display().to_string()),
        Err(err) => push("ffmpeg", false, format!("{err:#}")),
    }
    for blocker in &support.blockers {
        push(&blocker.name, false, blocker.detail.clone());
    }
    if !support.dictation_supported {
        return checks;
    }

    push(
        "Audio recorder",
        command_exists("pw-record"),
        "pw-record (PipeWire)".into(),
    );
    push(
        "Whisper",
        command_exists("whisper-cli"),
        "whisper-cli in PATH".into(),
    );
    match stt::detect_model_path() {
        Some(path) => push("Whisper model", true, path.display().to_string()),
        None => push(
            "Whisper model",
            false,
            "no ggml model in ~/.local/share/whisper.cpp/models".into(),
        ),
    }
    let ydotool = command_exists("ydotool");
    let socket = std::env::var("YDOTOOL_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/tmp/.ydotool_socket".into());
    let socket_ok = std::path::Path::new(&socket).exists();
    push(
        "Paste backend",
        ydotool && socket_ok,
        if !ydotool {
            "ydotool not installed".into()
        } else if !socket_ok {
            "ydotool installed, but ydotool.service socket not found".into()
        } else {
            "ydotool + ydotoold socket".into()
        },
    );
    push(
        "Clipboard",
        command_exists("wl-copy") || command_exists("xclip") || command_exists("xsel"),
        "wl-copy / xclip / xsel".into(),
    );
    let cleanup_policy = cleanup::CleanupPolicy::from_app_config(&cfg);
    let (cleanup_ok, cleanup_detail) = if cleanup_policy.provider == "none" {
        (true, "cleanup disabled — raw transcript is pasted".into())
    } else {
        match cleanup::resolve_policy(&cleanup_policy) {
            Err(error) => (false, error.to_string()),
            Ok(target) if target.provider == "custom" && target.model.is_empty() => {
                (false, "custom endpoint set — pick a model".into())
            }
            Ok(target) if target.provider == "custom" => {
                (true, format!("custom · {}", target.model))
            }
            Ok(target) if target.provider == "ollama" && target.model.ends_with(":cloud") => (
                true,
                format!(
                    "ollama · {} — note: ':cloud' models run on ollama.com",
                    target.model
                ),
            ),
            Ok(target) if target.provider == "ollama" => (true, "ollama (local)".into()),
            Ok(target) => (true, format!("{} ready", target.provider)),
        }
    };
    push("Cleanup provider", cleanup_ok, cleanup_detail);
    if cfg.general.local_only {
        push(
            "Privacy",
            true,
            "local-only mode on — no text leaves this machine".into(),
        );
    }
    checks
}

#[tauri::command]
async fn list_cleanup_models(config: AppConfig) -> CommandResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        pickscribe::engine::cleanup::list_models(&config).map_err(err_string)
    })
    .await
    .map_err(err_string)?
}

#[tauri::command]
fn list_models() -> Vec<String> {
    stt::available_models()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect()
}

#[tauri::command]
fn toggle_float_button(app: AppHandle) -> CommandResult<bool> {
    settings::toggle_float_button(&app).map(|config| config.general.float_button)
}

#[tauri::command]
fn copy_text(text: String) -> CommandResult<()> {
    pickscribe::engine::paste::copy_to_clipboard(&text).map_err(err_string)
}

#[tauri::command]
fn show_main_window(app: AppHandle) {
    focus_main_window(&app);
}

pub(crate) fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// tao's Wayland CSD wraps the header bar in a GtkEventBox with
// above-child input, which swallows clicks on the minimize/maximize/close
// buttons until a maximize/restore cycle re-stacks the input windows
// (tauri-apps/tao#1218). Lower the box below its child so the buttons get
// their events back.
#[cfg(target_os = "linux")]
fn fix_csd_titlebar_input(window: &tauri::WebviewWindow) {
    let handle = window.clone();
    let _ = window.run_on_main_thread(move || {
        use gtk::prelude::*;

        if let Ok(gtk_window) = handle.gtk_window()
            && let Some(titlebar) = gtk_window.titlebar()
            && let Some(event_box) = titlebar.downcast_ref::<gtk::EventBox>()
        {
            event_box.set_above_child(false);
        }
    });
}

// Shared Pickforge float-capsule geometry (kept in sync with PickGauge).
// On Linux the glow margin gives the capsule's box-shadow room to fade out
// inside the window, and the GTK input shape below keeps that transparent
// ring click-through. Other platforms have no input-shape equivalent, so
// they keep a snug window (and Float.svelte drops the outer glow there).
// Float.svelte's capsule margin must match FLOAT_GLOW_MARGIN per platform.
const FLOAT_CAPSULE_WIDTH: i32 = 204;
const FLOAT_CAPSULE_HEIGHT: i32 = 56;
#[cfg(target_os = "linux")]
const FLOAT_GLOW_MARGIN: i32 = 24;
#[cfg(not(target_os = "linux"))]
const FLOAT_GLOW_MARGIN: i32 = 2;
const FLOAT_WINDOW_WIDTH: i32 = FLOAT_CAPSULE_WIDTH + 2 * FLOAT_GLOW_MARGIN;
const FLOAT_WINDOW_HEIGHT: i32 = FLOAT_CAPSULE_HEIGHT + 2 * FLOAT_GLOW_MARGIN;

/// The floating capsule lives above every other window, never takes focus,
/// and opens the main window on click. Created lazily so disabling it in
/// settings simply hides the window.
///
/// Task-switcher/taskbar exclusion is platform-dependent — Tauri's
/// `skip_taskbar` and `focusable(false)` below are the portable baseline,
/// but they are not sufficient (or available) everywhere:
/// - **Linux/KDE (Wayland and its `PICKSCRIBE_X11=1` XWayland fallback)**:
///   `skip_taskbar` only sets the standard `_NET_WM_STATE_SKIP_TASKBAR`/
///   `SKIP_PAGER` X11 hints, which say nothing about KWin's Alt+Tab switcher,
///   and GTK's Wayland backend does not implement them at all.
///   `kwin::ensure_float_rule` additionally installs a KWin `skipswitcher`
///   window rule so the capsule is excluded from Alt+Tab too (see `kwin.rs`
///   for the full rationale).
/// - **Other Linux window managers**: only get the standards-based
///   `skip_taskbar`/`focusable(false)` hints above; there is no
///   app-managed, WM-specific configuration for non-KDE compositors.
/// - **Windows/macOS**: PickScribe ships Linux only for now (see README), so
///   there is no task-switcher behavior to document there yet.
pub(crate) fn ensure_float_window(app: &AppHandle, visible: bool) {
    if let Some(window) = app.get_webview_window("float") {
        if visible {
            let _ = window.show();
            clamp_float_window_size(&window);
        } else {
            let _ = window.hide();
        }
        return;
    }
    if !visible {
        return;
    }
    kwin::ensure_float_rule();
    let window = WebviewWindowBuilder::new(app, "float", WebviewUrl::App("index.html".into()))
        .title("PickScribe Float")
        .inner_size(
            f64::from(FLOAT_WINDOW_WIDTH),
            f64::from(FLOAT_WINDOW_HEIGHT),
        )
        .min_inner_size(
            f64::from(FLOAT_WINDOW_WIDTH),
            f64::from(FLOAT_WINDOW_HEIGHT),
        )
        .max_inner_size(
            f64::from(FLOAT_WINDOW_WIDTH),
            f64::from(FLOAT_WINDOW_HEIGHT),
        )
        // Resizable + exact min/max hints: with the decoration CSS reset in
        // clamp_float_window_size, GTK honors these as a fixed size on
        // Wayland (non-resizable windows ignore programmatic resizes there).
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .focusable(false)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .position(64.0, 64.0)
        .build();
    if let Ok(window) = window {
        clamp_float_window_size(&window);
    }
}

/// GTK won't size the capsule correctly on its own: WebKitGTK requests a
/// 200x200 minimum on X11, and on Wayland resizes issued before the surface
/// is mapped are dropped, collapsing the window to the webview's tiny natural
/// height. Clamp immediately and again shortly after mapping.
#[cfg(target_os = "linux")]
fn clamp_float_window_size(window: &tauri::WebviewWindow) {
    fn clamp_now(window: &tauri::WebviewWindow) {
        let window_handle = window.clone();
        let _ = window.run_on_main_thread(move || {
            use gtk::prelude::*;

            if let Ok(gtk_window) = window_handle.gtk_window() {
                gtk_window.set_size_request(FLOAT_WINDOW_WIDTH, FLOAT_WINDOW_HEIGHT);
                if let Some(child) = gtk_window.child() {
                    child.set_size_request(FLOAT_WINDOW_WIDTH, FLOAT_WINDOW_HEIGHT);
                }
                gtk_window.resize(FLOAT_WINDOW_WIDTH, FLOAT_WINDOW_HEIGHT);
                if let Some(gdk_window) = gtk_window.window() {
                    // 2px slack so the capsule border stays clickable.
                    let rect = gtk::cairo::RectangleInt::new(
                        FLOAT_GLOW_MARGIN - 2,
                        FLOAT_GLOW_MARGIN - 2,
                        FLOAT_CAPSULE_WIDTH + 4,
                        FLOAT_CAPSULE_HEIGHT + 4,
                    );
                    let region = gtk::cairo::Region::create_rectangle(&rect);
                    gdk_window.input_shape_combine_region(&region, 0, 0);
                }
            }
        });
    }

    // GTK reserves invisible CSD shadow/resize margins (~26px per side) on
    // undecorated Wayland windows, shrinking the visible capsule by 52px in
    // each axis. Strip the decoration node — but only for this window: a
    // screen-wide reset also desyncs the main window's CSD hit-testing, so
    // its titlebar buttons stop responding until a maximize re-syncs them.
    {
        let window_handle = window.clone();
        let _ = window.run_on_main_thread(move || {
            use gtk::prelude::*;

            if let Ok(gtk_window) = window_handle.gtk_window() {
                gtk_window.set_widget_name("pickforge-float");
            }
            let provider = gtk::CssProvider::new();
            let _ = provider.load_from_data(
                b"window#pickforge-float decoration{box-shadow:none;margin:0;padding:0;border:none;border-radius:0;}",
            );
            if let Some(screen) = gtk::gdk::Screen::default() {
                gtk::StyleContext::add_provider_for_screen(
                    &screen,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        });
    }

    clamp_now(window);
    // GTK3 CSD quirk: geometry hints are interpreted including the invisible
    // shadow margins while resize() works on content size, so a fixed hint
    // clamps the content too small (by the shadow size, theme-dependent).
    // Feedback loop: measure the content-size error and grow the hints until
    // the content settles at exactly the target.
    let window = window.clone();
    std::thread::spawn(move || {
        let compensation = std::sync::Arc::new(std::sync::Mutex::new((0i32, 0i32)));
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let handle = window.clone();
            let compensation = std::sync::Arc::clone(&compensation);
            let _ = window.run_on_main_thread(move || {
                use gtk::prelude::*;

                let Ok(gtk_window) = handle.gtk_window() else {
                    return;
                };
                let (content_w, content_h) = gtk_window.size();
                if content_w == FLOAT_WINDOW_WIDTH && content_h == FLOAT_WINDOW_HEIGHT {
                    return;
                }
                let mut comp = compensation.lock().unwrap();
                comp.0 = (comp.0 + FLOAT_WINDOW_WIDTH - content_w).clamp(0, 200);
                comp.1 = (comp.1 + FLOAT_WINDOW_HEIGHT - content_h).clamp(0, 200);
                let total_w = FLOAT_WINDOW_WIDTH + comp.0;
                let total_h = FLOAT_WINDOW_HEIGHT + comp.1;
                let geometry = gtk::gdk::Geometry::new(
                    total_w,
                    total_h,
                    total_w,
                    total_h,
                    0,
                    0,
                    0,
                    0,
                    0f64,
                    0f64,
                    gtk::gdk::Gravity::Center,
                );
                gtk_window.set_geometry_hints(
                    None::<&gtk::Window>,
                    Some(&geometry),
                    gtk::gdk::WindowHints::MIN_SIZE | gtk::gdk::WindowHints::MAX_SIZE,
                );
                gtk_window.resize(total_w, total_h);
            });
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn clamp_float_window_size(window: &tauri::WebviewWindow) {
    let _ = window.set_size(tauri::LogicalSize::new(
        f64::from(FLOAT_WINDOW_WIDTH),
        f64::from(FLOAT_WINDOW_HEIGHT),
    ));
}

pub fn run() {
    let context = tauri::generate_context!();
    let cfg = AppConfig::load();
    let sentry_enabled = settings::sentry_client_enabled(&cfg);
    let release = format!(
        "pickscribe@{}",
        context
            .config()
            .version
            .clone()
            .expect("version in tauri.conf.json")
    );
    let sentry_client = sentry::init((
        if sentry_enabled { SENTRY_DSN } else { "" },
        sentry::ClientOptions {
            release: Some(release.into()),
            transport: Some(settings::transport_factory()),
            before_send: Some(Arc::new(|mut event| {
                if !settings::telemetry_enabled() {
                    return None;
                }
                event.server_name = None;
                event.breadcrumbs = Default::default();
                strip_debug_image_paths(&mut event);
                Some(event)
            })),
            ..Default::default()
        },
    ));
    let sentry_client_handle = sentry_client
        .is_enabled()
        .then(|| sentry::Hub::main().client())
        .flatten();
    settings::initialize_reporting(sentry_enabled, sentry_client_handle);
    let sentry_plugin = if sentry_enabled {
        tauri_plugin_sentry::init(&sentry_client)
    } else {
        tauri_plugin_sentry::init_with_no_injection(&sentry_client)
    };
    let engine = Arc::new(Engine::new().expect("failed to open PickScribe data directory"));

    tauri::Builder::default()
        .manage(engine)
        .plugin(sentry_plugin)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle") {
                let engine = engine::engine(app);
                engine.set_chord_override(parse_chord_arg(&args));
                engine.toggle(app);
            } else {
                focus_main_window(app);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            get_state,
            toggle_dictation,
            cancel_dictation,
            get_app_config,
            update_app_config,
            get_platform_support,
            list_history,
            delete_history_entry,
            clear_history,
            transcribe_media_file,
            cancel_file_transcription,
            pick_media_file,
            export_history_entry,
            get_metrics,
            run_doctor,
            list_models,
            list_cleanup_models,
            toggle_float_button,
            get_system_theme,
            copy_text,
            show_main_window,
        ])
        .setup(|app| {
            tray::setup(app)?;
            let cfg = AppConfig::load();
            ensure_float_window(app.handle(), cfg.general.float_button);
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                fix_csd_titlebar_input(&window);
            }
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--hidden")
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.hide();
            }
            if args.iter().any(|a| a == "--toggle") {
                let handle = app.handle().clone();
                let engine = engine::engine(&handle);
                engine.set_chord_override(parse_chord_arg(&args));
                engine.toggle(&handle);
            }
            Ok(())
        })
        .build(context)
        .expect("error while building PickScribe")
        .run(|app_handle, event| match event {
            tauri::RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                api.prevent_close();
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            tauri::RunEvent::ExitRequested { code: None, api, .. } => {
                api.prevent_exit();
            }
            _ => {}
        });
}
