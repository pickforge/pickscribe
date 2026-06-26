mod engine;
mod kwin;
mod tray;

use std::sync::Arc;

use pickscribe::config::AppConfig;
use pickscribe::engine::{command_exists, stt};
use pickscribe::history::{HistoryEntry, Metrics};
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use engine::{Engine, StatePayload};

type CommandResult<T> = Result<T, String>;

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

#[tauri::command]
fn cancel_dictation(app: AppHandle, engine: State<'_, Arc<Engine>>) {
    engine.cancel(&app);
}

#[tauri::command]
fn get_app_config() -> AppConfig {
    AppConfig::load()
}

pub(crate) const EVENT_CONFIG: &str = "pickscribe://config";

#[tauri::command]
fn update_app_config(app: AppHandle, config: AppConfig) -> CommandResult<AppConfig> {
    use tauri::Emitter;
    config.save().map_err(err_string)?;
    ensure_float_window(&app, config.general.float_button);
    let _ = app.emit(EVENT_CONFIG, &config);
    Ok(config)
}

#[tauri::command]
fn get_system_theme() -> String {
    if tray::panel_prefers_dark() { "dark".into() } else { "light".into() }
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
fn run_doctor() -> Vec<DoctorCheck> {
    let cfg = AppConfig::load();
    let mut checks = Vec::new();
    let mut push = |name: &str, ok: bool, detail: String| {
        checks.push(DoctorCheck {
            name: name.into(),
            ok,
            detail,
        });
    };

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
    let provider = cfg.effective_provider();
    let (cleanup_ok, cleanup_detail) = if cfg.cleanup.provider == "none" {
        (true, "cleanup disabled — raw transcript is pasted".into())
    } else if cfg.general.local_only && matches!(provider.as_str(), "deepseek" | "openai") {
        (
            false,
            format!("local-only mode blocks {provider} — switch to Ollama, a local endpoint, or none"),
        )
    } else if provider == "custom" {
        if cfg.cleanup.endpoint.is_empty() {
            (false, "custom provider needs an endpoint URL".into())
        } else if cfg.general.local_only
            && !pickscribe::engine::cleanup::is_local_endpoint(&cfg.cleanup.endpoint)
        {
            (false, "local-only mode blocks this remote endpoint".into())
        } else if cfg.cleanup.model.is_empty() {
            (false, "custom endpoint set — pick a model".into())
        } else {
            (true, format!("custom · {}", cfg.cleanup.model))
        }
    } else if provider == "ollama" {
        if cfg.cleanup.model.ends_with(":cloud") {
            if cfg.general.local_only {
                (
                    false,
                    format!(
                        "{} is an Ollama cloud model (runs on ollama.com) — blocked in local-only mode",
                        cfg.cleanup.model
                    ),
                )
            } else {
                (
                    true,
                    format!("ollama · {} — note: ':cloud' models run on ollama.com", cfg.cleanup.model),
                )
            }
        } else {
            (true, "ollama (local)".into())
        }
    } else if cfg.resolve_api_key(&provider).is_some() {
        (true, format!("{provider} ready"))
    } else {
        (false, format!("{provider} selected but no API key found"))
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
fn list_cleanup_models(config: AppConfig) -> CommandResult<Vec<String>> {
    pickscribe::engine::cleanup::list_models(&config).map_err(err_string)
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
    let mut cfg = AppConfig::load();
    cfg.general.float_button = !cfg.general.float_button;
    cfg.save().map_err(err_string)?;
    ensure_float_window(&app, cfg.general.float_button);
    Ok(cfg.general.float_button)
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

        if let Ok(gtk_window) = handle.gtk_window() {
            if let Some(titlebar) = gtk_window.titlebar() {
                if let Some(event_box) = titlebar.downcast_ref::<gtk::EventBox>() {
                    event_box.set_above_child(false);
                }
            }
        }
    });
}

// Shared Pickforge float-capsule geometry (kept in sync with PickGauge).
const FLOAT_WINDOW_WIDTH: i32 = 208;
const FLOAT_WINDOW_HEIGHT: i32 = 60;

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
    let engine = Arc::new(Engine::new().expect("failed to open PickScribe data directory"));

    tauri::Builder::default()
        .manage(engine)
        .plugin(tauri_plugin_opener::init())
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
            list_history,
            delete_history_entry,
            clear_history,
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
            if args.iter().any(|a| a == "--hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            if args.iter().any(|a| a == "--toggle") {
                let handle = app.handle().clone();
                let engine = engine::engine(&handle);
                engine.set_chord_override(parse_chord_arg(&args));
                engine.toggle(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
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
