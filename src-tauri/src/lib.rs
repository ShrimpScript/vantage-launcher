mod error;
mod install;
mod java;
mod launch;
mod jar;
mod auth;
mod meta;
mod modrinth;
mod pack;
mod net;
mod store;

use error::{Error, Result};
use serde::Serialize;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    http: reqwest::Client,
    store: store::Store,
    manifest: tokio::sync::Mutex<Option<meta::Manifest>>,
}

impl AppState {
    async fn manifest(&self) -> Result<meta::Manifest> {
        let mut guard = self.manifest.lock().await;
        if let Some(m) = guard.as_ref() {
            return Ok(m.clone());
        }
        let fetched = meta::manifest(&self.http).await?;
        *guard = Some(fetched.clone());
        Ok(fetched)
    }

    async fn entry(&self, id: &str) -> Result<meta::Entry> {
        self.manifest()
            .await?
            .versions
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| Error::UnknownVersion(id.to_string()))
    }
}

#[derive(Serialize)]
pub struct VersionsOut {
    latest: meta::Latest,
    releases: Vec<meta::Entry>,
    snapshots: Vec<meta::Entry>,
    total: usize,
}

#[tauri::command]
async fn versions(state: State<'_, AppState>) -> Result<VersionsOut> {
    let m = state.manifest().await?;
    let total = m.versions.len();
    let releases: Vec<_> = m.versions.iter().filter(|v| v.kind == "release").take(30).cloned().collect();
    let snapshots: Vec<_> = m.versions.iter().filter(|v| v.kind == "snapshot").take(10).cloned().collect();
    Ok(VersionsOut { latest: m.latest, releases, snapshots, total })
}

#[derive(Serialize)]
pub struct Inspection {
    id: String,
    main_class: String,
    java: Option<meta::JavaVersion>,
    asset_index_id: String,
    asset_objects: usize,
    asset_bytes: u64,
    client_bytes: u64,
    libs_total: usize,
    libs_applicable: usize,
    os: String,
    installed: bool,
}

/// Everything the Play screen needs to tell the truth about a version before you commit
/// to downloading half a gigabyte.
#[tauri::command]
async fn inspect(state: State<'_, AppState>, id: String) -> Result<Inspection> {
    let entry = state.entry(&id).await?;
    let detail = meta::detail(&state.http, &entry.url).await?;

    let raw = state.http.get(&detail.asset_index.url).send().await?.error_for_status()?.bytes().await?;
    let index: meta::AssetIndex = serde_json::from_slice(&raw)?;

    let libs_applicable = detail.libraries.iter().filter(|l| l.applies()).count();
    Ok(Inspection {
        id: detail.id.clone(),
        main_class: detail.main_class.clone(),
        java: detail.java_version.clone(),
        asset_index_id: detail.asset_index.id.clone(),
        asset_objects: index.objects.len(),
        asset_bytes: detail.asset_index.total_size,
        client_bytes: detail.downloads.client.size,
        libs_total: detail.libraries.len(),
        libs_applicable,
        os: meta::os_name().to_string(),
        installed: state.store.client_jar(&detail.id).exists(),
    })
}

#[derive(Clone, Serialize)]
struct Progress {
    phase: &'static str,
    done: u64,
    total: u64,
    bytes: u64,
    total_bytes: u64,
    skipped: u64,
}

/// Emits `install:progress` to the window. The UI never polls.
struct WindowSink(AppHandle);

impl install::ProgressSink for WindowSink {
    fn emit(&self, phase: &'static str, done: u64, total: u64, bytes: u64, total_bytes: u64, skipped: u64) {
        let _ = self.0.emit(
            "install:progress",
            Progress { phase, done, total, bytes, total_bytes, skipped },
        );
    }
}

#[tauri::command]
async fn install(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<install::Report> {
    let entry = state.entry(&id).await?;
    install::run(&state.http, &state.store, &entry, Arc::new(WindowSink(app))).await
}

/// A block texture from the selected version's own jar, as a data URI. Not stock art —
/// this is the user's copy of the game, for the exact version they picked.
#[tauri::command]
fn texture(state: State<'_, AppState>, id: String, name: String) -> Result<String> {
    // Path is built here, never taken from the UI, so a crafted name cannot escape the jar.
    let safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    jar::read_png_data_uri(
        &state.store.client_jar(&id),
        &format!("assets/minecraft/textures/block/{safe}.png"),
    )
}

/// The game's own default skin, straight out of the jar. Honest stand-in until sign-in.
#[tauri::command]
fn default_skin(state: State<'_, AppState>, id: String) -> Result<String> {
    let path = &state.store.client_jar(&id);
    for candidate in [
        "assets/minecraft/textures/entity/player/wide/steve.png",
        "assets/minecraft/textures/entity/player/slim/alex.png",
        "assets/minecraft/textures/entity/steve.png",
    ] {
        if let Ok(uri) = jar::read_png_data_uri(path, candidate) {
            return Ok(uri);
        }
    }
    Err(Error::Other("no default skin in this version's jar".into()))
}

#[tauri::command]
fn block_list(state: State<'_, AppState>, id: String) -> Result<Vec<String>> {
    jar::block_textures(&state.store.client_jar(&id))
}


/* ── Modrinth: search, install, list ─────────────────────────────────────── */

#[tauri::command]
async fn mod_search(
    state: State<'_, AppState>,
    query: String,
    game_version: String,
) -> Result<Vec<modrinth::Hit>> {
    modrinth::search(&state.http, &query, &game_version, 20).await
}

#[derive(Serialize)]
pub struct InstalledMod {
    filename: String,
    bytes: u64,
}

#[tauri::command]
fn mods_installed(state: State<'_, AppState>) -> Vec<InstalledMod> {
    let dir = state.store.profile_mods("main");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<InstalledMod> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jar") {
                return None;
            }
            let bytes = e.metadata().ok()?.len();
            Some(InstalledMod { filename: name, bytes })
        })
        .collect();
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out
}

#[derive(Serialize)]
pub struct ModInstalled {
    filename: String,
    version_number: String,
    version_type: String,
    bytes: u64,
}

/// Resolve the newest build compatible with this game version and install it, sha1-verified.
/// Nothing is repackaged: the file comes from cdn.modrinth.com exactly as its author published it.
#[tauri::command]
async fn mod_install(
    state: State<'_, AppState>,
    project: String,
    game_version: String,
) -> Result<ModInstalled> {
    let versions = modrinth::versions(&state.http, &project, &game_version).await?;
    let ver = versions
        .into_iter()
        .next()
        .ok_or_else(|| Error::Other(format!("{project} has no Fabric build for {game_version}")))?;
    let file = ver
        .primary()
        .ok_or_else(|| Error::Other(format!("{project} {} has no downloadable file", ver.version_number)))?
        .clone();

    let dest = state.store.profile_mods("main").join(&file.filename);
    net::fetch_all(
        &state.http,
        vec![net::Item {
            url: file.url.clone(),
            dest,
            sha1: Some(file.hashes.sha1.clone()),
            size: file.size,
        }],
        Arc::new(net::Counters::default()),
    )
    .await?;

    Ok(ModInstalled {
        filename: file.filename,
        version_number: ver.version_number,
        version_type: ver.version_type,
        bytes: file.size,
    })
}

#[tauri::command]
fn mod_remove(state: State<'_, AppState>, filename: String) -> Result<()> {
    // The name is only ever used as a leaf, never joined from UI input as a path.
    let leaf = std::path::Path::new(&filename)
        .file_name()
        .ok_or_else(|| Error::Other("not a file name".into()))?;
    std::fs::remove_file(state.store.profile_mods("main").join(leaf))?;
    Ok(())
}


/* ── the Vantage Set ─────────────────────────────────────────────────────── */

#[derive(Serialize)]
pub struct SetView {
    members: Vec<SetMember>,
    total_bytes: u64,
    applied: bool,
    loader: String,
    version_id: String,
}

#[derive(Serialize)]
pub struct SetMember {
    #[serde(flatten)]
    member: pack::Member,
    installed: bool,
}

/// Resolve the Set against Modrinth and report, per member, whether the exact pinned file is
/// already on disk. Exact filename comparison — not the slug heuristic the search list uses.
#[tauri::command]
async fn set_status(state: State<'_, AppState>, game_version: String) -> Result<SetView> {
    let r = pack::resolve(&state.http, &game_version).await?;
    let dir = state.store.profile_mods("main");
    let members: Vec<SetMember> = r
        .members
        .into_iter()
        .map(|m| {
            let installed = dir.join(&m.filename).exists();
            SetMember { member: m, installed }
        })
        .collect();
    Ok(SetView {
        applied: !members.is_empty() && members.iter().all(|m| m.installed),
        total_bytes: r.total_bytes,
        loader: r.index.dependencies.get("fabric-loader").cloned().unwrap_or_default(),
        version_id: r.index.version_id.clone(),
        members,
    })
}

#[derive(Serialize)]
pub struct SetReport {
    installed: usize,
    bytes: u64,
    mrpack: String,
    seconds: f64,
}

#[tauri::command]
async fn set_apply(state: State<'_, AppState>, game_version: String) -> Result<SetReport> {
    let started = std::time::Instant::now();
    let r = pack::resolve(&state.http, &game_version).await?;
    let bytes = pack::apply(&state.http, &state.store, &r.index).await?;
    let mrpack = pack::export(&state.store, &r.index)?;
    Ok(SetReport {
        installed: r.index.files.len(),
        bytes,
        mrpack: mrpack.display().to_string(),
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// Removing the Set must be as easy as applying it, or it is a cage (DESIGN.md §8).
#[tauri::command]
async fn set_remove(state: State<'_, AppState>, game_version: String) -> Result<usize> {
    let r = pack::resolve(&state.http, &game_version).await?;
    let dir = state.store.profile_mods("main");
    let mut removed = 0;
    for f in &r.index.files {
        let leaf = f.path.trim_start_matches("mods/");
        if std::fs::remove_file(dir.join(leaf)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}


/* ── Microsoft sign-in ───────────────────────────────────────────────────── */

#[derive(Serialize)]
pub struct AuthStatus {
    configured: bool,
    /// Where to drop the client ID. Shown in Settings so it is discoverable without docs.
    client_id_path: String,
    account: Option<auth::Account>,
}

#[tauri::command]
fn auth_status(state: State<'_, AppState>) -> AuthStatus {
    AuthStatus {
        configured: auth::client_id(&state.store.root).is_some(),
        client_id_path: state.store.root.join("client-id.txt").display().to_string(),
        account: None,
    }
}

#[tauri::command]
async fn sign_in(state: State<'_, AppState>) -> Result<auth::Account> {
    let id = auth::client_id(&state.store.root).ok_or_else(|| {
        Error::Other(format!(
            "No Azure client ID yet. Put one in {}",
            state.store.root.join("client-id.txt").display()
        ))
    })?;
    auth::sign_in(&state.http, &id).await
}

#[derive(Serialize)]
pub struct StoreInfo {
    root: String,
    files: u64,
    bytes: u64,
    versions: Vec<store::Installed>,
}

#[tauri::command]
fn store_info(state: State<'_, AppState>) -> StoreInfo {
    let (files, bytes) = state.store.usage();
    StoreInfo {
        root: state.store.root.display().to_string(),
        files,
        bytes,
        versions: state.store.installed_versions(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(AppState {
                http: net::client()?,
                store: store::Store::discover()?,
                manifest: tokio::sync::Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            versions, inspect, install, store_info, texture, default_skin, block_list,
            mod_search, mod_install, mods_installed, mod_remove,
            set_status, set_apply, set_remove,
            auth_status, sign_in
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vantage");
}

/// `vantage-launcher --install <version>`: the same pipeline the button runs, without a
/// window. Exists so the download core can be exercised in CI and on a headless box.
pub fn headless_install(id: &str) {
    struct Stdout;
    impl install::ProgressSink for Stdout {
        fn emit(&self, phase: &'static str, done: u64, total: u64, bytes: u64, total_bytes: u64, skipped: u64) {
            let pct = if total > 0 { done * 100 / total } else { 0 };
            eprint!(
                "\r{phase:<7} {done:>6}/{total:<6} {pct:>3}%  {:>7.1}/{:>7.1} MB  skipped {skipped}   ",
                bytes as f64 / 1048576.0,
                total_bytes as f64 / 1048576.0
            );
        }
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let outcome = rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        let manifest = meta::manifest(&http).await?;
        let entry = manifest
            .versions
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| Error::UnknownVersion(id.to_string()))?;
        install::run(&http, &store, &entry, Arc::new(Stdout)).await
    });

    match outcome {
        Ok(r) => {
            eprintln!();
            println!(
                "{} ready: {} files ({} downloaded, {} already in store), {:.1} MB over the wire in {:.1}s ({:.0} files/s)",
                r.id,
                r.files,
                r.downloaded,
                r.skipped,
                r.bytes as f64 / 1048576.0,
                r.seconds,
                r.files as f64 / r.seconds.max(0.001)
            );
            println!("libraries: {} of {} applicable here", r.libs_applicable, r.libs_total);
            println!("store: {}", r.store_root);
        }
        Err(e) => {
            eprintln!();
            eprintln!("install failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--mods <game_version> <query>`: the same search the Mods screen runs, no window.
pub fn headless_mods(game_version: &str, query: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(async {
        let http = net::client()?;
        modrinth::search(&http, query, game_version, 8).await
    }) {
        Ok(hits) => {
            println!("{} results for \"{query}\" on Fabric {game_version}:", hits.len());
            for h in hits {
                println!("  {:<28} {:>12} downloads  {}", h.slug, h.downloads, h.title);
            }
        }
        Err(e) => {
            eprintln!("search failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--add <game_version> <project>`: resolve the newest Fabric build and install it, verified.
pub fn headless_add(game_version: &str, project: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        let versions = modrinth::versions(&http, project, game_version).await?;
        let ver = versions
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other(format!("{project} has no Fabric build for {game_version}")))?;
        let file = ver
            .primary()
            .ok_or_else(|| Error::Other("no downloadable file".into()))?
            .clone();
        let dest = store.profile_mods("main").join(&file.filename);
        net::fetch_all(
            &http,
            vec![net::Item {
                url: file.url.clone(),
                dest: dest.clone(),
                sha1: Some(file.hashes.sha1.clone()),
                size: file.size,
            }],
            Arc::new(net::Counters::default()),
        )
        .await?;
        Ok::<_, Error>((ver.version_number, ver.version_type, file, dest))
    }) {
        Ok((num, kind, file, dest)) => {
            println!("installed {project} {num} ({kind})");
            println!("  {} — {:.2} MB, sha1 {}", file.filename, file.size as f64 / 1048576.0, &file.hashes.sha1[..16]);
            println!("  -> {}", dest.display());
        }
        Err(e) => {
            eprintln!("add failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--set <game_version>`: resolve the Set, install it, and write the .mrpack. No window.
pub fn headless_set(game_version: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        let r = pack::resolve(&http, game_version).await?;
        let bytes = pack::apply(&http, &store, &r.index).await?;
        let out = pack::export(&store, &r.index)?;
        Ok::<_, Error>((r, bytes, out))
    }) {
        Ok((r, bytes, out)) => {
            println!("The Vantage Set — Minecraft {game_version}, Fabric {}",
                r.index.dependencies.get("fabric-loader").map(String::as_str).unwrap_or("?"));
            for m in &r.members {
                let kind = if m.version_type == "release" { String::new() } else { format!(" ({})", m.version_type) };
                println!("  {:<16} {:<12} {}{}", m.slug, m.role, m.version_number, kind);
            }
            println!("{} files, {:.1} MB installed", r.index.files.len(), bytes as f64 / 1048576.0);
            println!("mrpack: {}", out.display());
        }
        Err(e) => {
            eprintln!("set failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--auth-status`: is a client ID configured, and where does it go?
pub fn headless_auth_status() {
    let store = store::Store::discover().expect("store");
    let path = store.root.join("client-id.txt");
    match auth::client_id(&store.root) {
        Some(id) => {
            let shown = if id.len() > 8 { format!("{}…{}", &id[..4], &id[id.len() - 4..]) } else { id };
            println!("client ID configured: {shown}");
            println!("sign-in will work once the Azure app is approved for the Minecraft API");
        }
        None => {
            println!("no client ID configured");
            println!("  put it in: {}", path.display());
            println!("  or set:    VANTAGE_CLIENT_ID");
            println!("  apply for Minecraft API permission: https://aka.ms/mce-reviewappid");
        }
    }
}

/// `--launch <version> [name]`: assemble the command line and start the game.
/// Uses an offline session until Microsoft approves the app — singleplayer works,
/// online servers correctly refuse it.
pub fn headless_launch(id: &str, name: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let outcome = rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        let session = launch::Session::offline(name);
        let (plan, dir) = launch::plan(&http, &store, id, &session, 4096).await?;
        Ok::<_, Error>((plan, dir, session))
    });

    let (plan, dir, session) = match outcome {
        Ok(v) => v,
        Err(e) => {
            eprintln!("could not prepare launch: {e}");
            std::process::exit(1);
        }
    };

    println!("java        {}", plan.java);
    println!("main class  {}", plan.main_class);
    println!("classpath   {} entries", plan.classpath_entries);
    println!("game dir    {}", plan.game_dir);
    println!("session     {} ({}) — offline", session.name, &session.uuid[..8]);
    println!("jvm args    {}", plan.jvm_args.len());
    println!("game args   {}", plan.game_args.len());

    let mut child = match launch::spawn(&plan, std::path::Path::new(&dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!("\nstarted, pid {}", child.id());

    // Surface the first lines of game output so a failure is visible rather than silent.
    use std::io::{BufRead, BufReader};
    if let Some(out) = child.stdout.take() {
        let reader = BufReader::new(out);
        for line in reader.lines().take(14).map_while(|l| l.ok()) {
            println!("  | {line}");
        }
    }
    println!("(still running — close the game window to exit)");
    let _ = child.wait();
}
