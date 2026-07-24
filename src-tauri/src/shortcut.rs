use tauri::AppHandle;

#[cfg(target_os = "macos")]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

#[cfg(target_os = "macos")]
pub(crate) fn register_startup(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    if shortcut.is_empty() {
        return Ok(());
    }
    app.global_shortcut()
        .register(shortcut)
        .map_err(|err| format!("failed to register global shortcut {shortcut:?}: {err}"))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn register_startup(_app: &AppHandle, _shortcut: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn replace(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }

    let shortcuts = app.global_shortcut();
    if !new.is_empty() {
        shortcuts.register(new).map_err(|err| {
            format!("failed to register global shortcut {new:?}; the previous shortcut is still active: {err}")
        })?;
    }

    if !old.is_empty()
        && shortcuts.is_registered(old)
        && let Err(err) = shortcuts.unregister(old)
    {
        if !new.is_empty() {
            let _ = shortcuts.unregister(new);
        }
        return Err(format!(
            "failed to replace global shortcut {old:?} with {new:?}; the previous shortcut is still active: {err}"
        ));
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn replace(_app: &AppHandle, _old: &str, _new: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use tauri_plugin_global_shortcut::Shortcut;

    #[test]
    fn default_macos_shortcut_is_a_valid_accelerator() {
        assert!("Cmd+Shift+Space".parse::<Shortcut>().is_ok());
    }
}
