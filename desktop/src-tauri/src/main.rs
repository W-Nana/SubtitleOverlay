fn main() {
    configure_linux_webkit();
    subtitle_overlay_desktop::run()
}

#[cfg(target_os = "linux")]
fn configure_linux_webkit() {
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_webkit() {}
