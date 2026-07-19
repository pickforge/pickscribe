use std::sync::Mutex;
use std::time::{Duration, Instant};

use pickscribe::config::AppConfig;
use tauri::AppHandle;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::engine::Stage;

const ICON_IDLE_DARK: &[u8] = include_bytes!("../../assets/branding/pickscribe-tray-idle.png");
const ICON_RECORDING_DARK: &[u8] =
    include_bytes!("../../assets/branding/pickscribe-tray-recording.png");
const ICON_IDLE_LIGHT: &[u8] =
    include_bytes!("../../assets/branding/pickscribe-tray-idle-light.png");
const ICON_RECORDING_LIGHT: &[u8] =
    include_bytes!("../../assets/branding/pickscribe-tray-recording-light.png");

pub const TRAY_ID: &str = "pickscribe-tray";

/// Cached dark-mode probe: probing spawns a `gdbus` subprocess, and tray
/// sync runs 3-4x per segment during incremental transcription, so the
/// probe result is reused until the TTL lapses or the frontend reports an
/// explicit theme event via `refresh_panel_prefers_dark`.
static PANEL_DARK_CACHE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);
const PANEL_DARK_TTL: Duration = Duration::from_secs(30);

/// Last icon variant applied to the tray: (is_idle, dark_panel). Used to
/// skip redundant `set_icon` calls when the stage icon is unchanged.
static LAST_ICON: Mutex<Option<(bool, bool)>> = Mutex::new(None);

/// Whether the desktop prefers a dark color scheme (light tray strokes).
/// Serves the cached probe result when it is fresh enough.
pub(crate) fn panel_prefers_dark() -> bool {
    if let Ok(cache) = PANEL_DARK_CACHE.lock()
        && let Some((value, probed_at)) = *cache
        && probed_at.elapsed() < PANEL_DARK_TTL
    {
        return value;
    }
    refresh_panel_prefers_dark()
}

/// Force a fresh probe (used on explicit theme events from the frontend)
/// and update the cache the tray sync path reads.
pub(crate) fn refresh_panel_prefers_dark() -> bool {
    let value = probe_panel_prefers_dark();
    if let Ok(mut cache) = PANEL_DARK_CACHE.lock() {
        *cache = Some((value, Instant::now()));
    }
    value
}

/// Asks the XDG settings portal: 0 = no preference, 1 = dark, 2 = light.
/// Defaults to dark on any failure, matching the original icon set.
fn probe_panel_prefers_dark() -> bool {
    let output = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            !String::from_utf8_lossy(&out.stdout).contains("uint32 2")
        }
        _ => true,
    }
}

fn icon_bytes(idle: bool, dark_panel: bool) -> &'static [u8] {
    match (idle, dark_panel) {
        (true, true) => ICON_IDLE_DARK,
        (true, false) => ICON_IDLE_LIGHT,
        (false, true) => ICON_RECORDING_DARK,
        (false, false) => ICON_RECORDING_LIGHT,
    }
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let toggle_item = MenuItem::with_id(app, "toggle", "Start dictation", true, None::<&str>)?;
    let cancel_item = MenuItem::with_id(app, "cancel", "Cancel recording", true, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", "Open PickScribe", true, None::<&str>)?;
    let float_item =
        MenuItem::with_id(app, "float", "Show/hide floating button", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle_item, &cancel_item, &show_item, &float_item, &quit_item],
    )?;

    let variant = (true, panel_prefers_dark());
    *LAST_ICON.lock().unwrap() = Some(variant);
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("PickScribe — idle")
        .icon(tauri::image::Image::from_bytes(icon_bytes(variant.0, variant.1))?)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => {
                let engine = crate::engine::engine(app);
                engine.set_chord_override(None);
                engine.toggle(app);
            }
            "cancel" => crate::engine::engine(app).cancel(app),
            "show" => crate::focus_main_window(app),
            "float" => {
                let mut cfg = AppConfig::load();
                cfg.general.float_button = !cfg.general.float_button;
                let _ = cfg.save();
                crate::ensure_float_window(app, cfg.general.float_button);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let engine = crate::engine::engine(app);
                engine.set_chord_override(None);
                engine.toggle(app);
            }
        })
        .build(app)?;
    Ok(())
}

pub fn sync(app: &AppHandle, stage: Stage) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = match stage {
        Stage::Idle => "PickScribe — idle",
        Stage::Recording => "PickScribe — recording",
        Stage::Transcribing => "PickScribe — transcribing",
        Stage::Cleaning => "PickScribe — cleaning",
        Stage::Pasting => "PickScribe — pasting",
    };
    let variant = (matches!(stage, Stage::Idle), panel_prefers_dark());
    let changed = {
        let mut last = LAST_ICON.lock().unwrap();
        if *last == Some(variant) {
            false
        } else {
            *last = Some(variant);
            true
        }
    };
    if changed && let Ok(icon) = tauri::image::Image::from_bytes(icon_bytes(variant.0, variant.1)) {
        let _ = tray.set_icon(Some(icon));
    }
    let _ = tray.set_tooltip(Some(tooltip));
}
