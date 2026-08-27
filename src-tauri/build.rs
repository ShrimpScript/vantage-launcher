fn main() {
    // Stamp the build time so `--version` can prove which binary is running. Diagnosing
    // "is this actually the new build?" by hand wastes more time than the stamp costs.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=VANTAGE_BUILD_EPOCH={now}");
    println!("cargo:rerun-if-changed=src");
    tauri_build::build()
}
