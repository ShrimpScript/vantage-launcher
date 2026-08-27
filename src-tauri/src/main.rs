// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMABUF renderer crashes with "Wayland protocol error 71" on NVIDIA
    // drivers, which is a large share of the PvP audience on Linux. Every Tauri app hits
    // this; most ship a wiki page telling users to export it themselves. We just set it,
    // before GTK initialises, and only when the user has not chosen otherwise.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--install" {
        vantage_launcher_lib::headless_install(&args[2]);
        return;
    }
    if args.len() >= 4 && args[1] == "--mods" {
        vantage_launcher_lib::headless_mods(&args[2], &args[3]);
        return;
    }
    if args.len() >= 2 && args[1] == "--auth-status" {
        vantage_launcher_lib::headless_auth_status();
        return;
    }
    if args.len() >= 3 && args[1] == "--set" {
        vantage_launcher_lib::headless_set(&args[2]);
        return;
    }
    if args.len() >= 4 && args[1] == "--add" {
        vantage_launcher_lib::headless_add(&args[2], &args[3]);
        return;
    }

    vantage_launcher_lib::run()
}
