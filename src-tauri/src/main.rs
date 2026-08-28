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

    if args.len() >= 2 && (args[1] == "--help" || args[1] == "-h") {
        print!("{}", vantage_launcher_lib::HELP);
        return;
    }
    if args.len() >= 2 && (args[1] == "--version" || args[1] == "-V") {
        println!("vantage {}", env!("CARGO_PKG_VERSION"));
        let epoch: i64 = env!("VANTAGE_BUILD_EPOCH").parse().unwrap_or(0);
        // Plain UTC from the epoch, no date crate for one line.
        let (mut d, secs) = (epoch / 86_400, epoch % 86_400);
        let (mut y, mut m2) = (1970, 1);
        loop {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            let len = if leap { 366 } else { 365 };
            if d < len { break; }
            d -= len; y += 1;
        }
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let months = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for len in months {
            if d < len { break; }
            d -= len; m2 += 1;
        }
        println!(
            "built {y:04}-{m2:02}-{:02} {:02}:{:02} UTC",
            d + 1, secs / 3600, (secs % 3600) / 60
        );
        return;
    }
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
    if args.len() >= 3 && args[1] == "--launch" {
        let name = args.get(3).map(String::as_str).unwrap_or("Player");
        vantage_launcher_lib::headless_launch(&args[2], name);
        return;
    }
    if args.len() >= 5 && args[1] == "--pack" {
        vantage_launcher_lib::headless_pack(&args[2], &args[3], &args[4]);
        return;
    }
    if args.len() >= 3 && args[1] == "--set" {
        vantage_launcher_lib::headless_set(&args[2]);
        return;
    }
    if args.len() >= 2 && args[1] == "--client" {
        vantage_launcher_lib::headless_client(args.get(2).map(|s| s.as_str()));
        return;
    }
    if args.len() >= 4 && args[1] == "--add" {
        vantage_launcher_lib::headless_add(&args[2], &args[3]);
        return;
    }

    vantage_launcher_lib::run()
}
