#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The floating button relies on always-on-top, which GTK can only set on
    // X11. Run under XWayland on Wayland sessions unless explicitly disabled.
    if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        && std::env::var_os("PICKSCRIBE_NATIVE_WAYLAND").is_none()
        && std::env::var_os("GDK_BACKEND").is_none()
    {
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    pickscribe_app_lib::run()
}
