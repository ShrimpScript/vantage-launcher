mod client;
mod error;
mod fabric;
mod install;
mod java;
mod launch;
mod jar;
mod accounts;
mod auth;
mod manifest;
mod meta;
mod modrinth;
mod pack;
mod net;
mod store;
mod video;

use client::ClientStatus;
use error::{Error, Result};
use serde::Serialize;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

pub const HELP: &str = "\
vantage — a Minecraft: Java Edition launcher

    vantage                            open the launcher

Versions
    vantage --install <version>        download and verify a version
    vantage --launch <version> [name]  start the game (offline session until
                                       Microsoft sign-in; without a name, plays
                                       as whoever the Accounts screen selected)

Content
    vantage --mods <version> <query>   search Modrinth for mods
    vantage --add <version> <project>  install one mod
    vantage --pack <version> <kind> <project>
                                       install a resource pack or shader
                                       kind: resourcepack | shader
    vantage --set <version>            apply the Vantage Set and write the .mrpack

The in-game client
    vantage --client                   which build is installed
    vantage --client <path|url>        install that jar as the client

Video
    vantage --video-defaults           VSync off, unlimited frames, max FOV,
                                       GUI scale 3 (a new profile gets these
                                       automatically on its first launch)

Account
    vantage --auth-status              is a Microsoft client ID configured

    vantage --help                     this
    vantage --version                  version

Everything lives in ~/.local/share/vantage, in the vanilla layout, so other
launchers can read it.
";

pub struct AppState {
    http: reqwest::Client,
    store: store::Store,
    manifest: tokio::sync::Mutex<Option<meta::Manifest>>,
    /// pid of the running game, if any. The launcher should say when the game is up.
    running: std::sync::Mutex<Option<u32>>,
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
    modrinth::search(&state.http, &query, &game_version, modrinth::Kind::Mod, 20).await
}

/* ── resource packs and shaders ──────────────────────────────────────────── */

/// Delete the previous file for this project, if the new install supersedes it.
fn drop_superseded(store: &store::Store, kind: &str, project: &str, new_file: &str, dir: &std::path::Path) {
    if let Some(old) = manifest::superseded(store, "main", kind, project, new_file) {
        let _ = std::fs::remove_file(dir.join(old));
    }
}

fn kind_of(s: &str) -> Result<modrinth::Kind> {
    match s {
        "resourcepack" => Ok(modrinth::Kind::ResourcePack),
        "shader" => Ok(modrinth::Kind::Shader),
        "mod" => Ok(modrinth::Kind::Mod),
        other => Err(Error::Other(format!("unknown kind: {other}"))),
    }
}

#[tauri::command]
async fn pack_search(
    state: State<'_, AppState>,
    query: String,
    game_version: String,
    kind: String,
) -> Result<Vec<modrinth::Hit>> {
    let k = kind_of(&kind)?;
    // An empty query on a pack browse should show the popular ones, not nothing.
    modrinth::search(&state.http, &query, &game_version, k, 20).await
}

#[derive(Serialize)]
pub struct PackInstalled {
    filename: String,
    bytes: u64,
    version_number: String,
}

/// Download once into the content-addressed pack store, then hard-link it into the profile.
/// The same pack enabled in several profiles costs one copy on disk.
#[tauri::command]
async fn pack_install(
    state: State<'_, AppState>,
    project: String,
    game_version: String,
    kind: String,
) -> Result<PackInstalled> {
    let k = kind_of(&kind)?;
    let versions = modrinth::versions_of(&state.http, &project, &game_version, k).await?;
    let ver = versions.into_iter().next().ok_or_else(|| {
        Error::Other(format!("{project} has no build for {game_version}"))
    })?;
    let file = ver
        .primary()
        .ok_or_else(|| Error::Other("no downloadable file".into()))?
        .clone();

    let blob = state.store.pack_blob(&file.hashes.sha1);
    net::fetch_all(
        &state.http,
        vec![net::Item {
            url: file.url.clone(),
            dest: blob.clone(),
            sha1: Some(file.hashes.sha1.clone()),
            size: file.size,
        }],
        Arc::new(net::Counters::default()),
    )
    .await?;

    let dir = state.store.profile_sub("main", k.profile_dir());
    std::fs::create_dir_all(&dir)?;
    let link = dir.join(&file.filename);
    let _ = std::fs::remove_file(&link);
    // Hard link where the filesystem allows it; fall back to a copy across devices.
    if std::fs::hard_link(&blob, &link).is_err() {
        std::fs::copy(&blob, &link)?;
    }

    drop_superseded(&state.store, &kind, &project, &file.filename, &dir);
    let meta = modrinth::detail(&state.http, &project).await.ok();
    manifest::record(
        &state.store, "main", &kind, &project, &file.filename,
        meta.as_ref().map_or("", |d| d.title.as_str()),
        meta.as_ref().and_then(|d| d.icon_url.clone()),
    )?;
    Ok(PackInstalled {
        filename: file.filename,
        bytes: file.size,
        version_number: ver.version_number,
    })
}

#[tauri::command]
fn packs_installed(state: State<'_, AppState>, kind: String) -> Result<Vec<InstalledMod>> {
    let k = kind_of(&kind)?;
    Ok(enrich(&state.store, &kind, state.store.profile_sub("main", k.profile_dir()), ".zip"))
}

/// The full project page for the detail view.
#[tauri::command]
async fn project_detail(state: State<'_, AppState>, project: String) -> Result<modrinth::Detail> {
    modrinth::detail(&state.http, &project).await
}

/// Exactly which project ids are installed for this kind. No slug-versus-filename guessing.
#[tauri::command]
fn installed_ids(state: State<'_, AppState>, kind: String) -> Vec<String> {
    manifest::installed_ids(&state.store, "main", &kind)
}

#[tauri::command]
fn pack_remove(state: State<'_, AppState>, kind: String, filename: String) -> Result<()> {
    let k = kind_of(&kind)?;
    let leaf = std::path::Path::new(&filename)
        .file_name()
        .ok_or_else(|| Error::Other("not a file name".into()))?;
    // Only the profile link goes; the blob stays for other profiles.
    std::fs::remove_file(state.store.profile_sub("main", k.profile_dir()).join(leaf))?;
    manifest::forget_file(&state.store, "main", &kind, &filename)?;
    Ok(())
}

#[derive(Serialize)]
pub struct InstalledMod {
    filename: String,
    bytes: u64,
    title: String,
    icon_url: Option<String>,
    project: Option<String>,
}

/// Files on disk, enriched from the manifest where we know what they are. A jar dropped in
/// by hand still shows up — it just has no title or icon, which is honest.
fn enrich(store: &store::Store, kind: &str, dir: std::path::PathBuf, ext: &str) -> Vec<InstalledMod> {
    let known = manifest::list(store, "main", kind);
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<InstalledMod> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(ext) {
                return None;
            }
            let hit = known.iter().find(|(_, v)| v.file() == name);
            Some(InstalledMod {
                bytes: e.metadata().ok()?.len(),
                title: hit.map_or_else(|| name.clone(), |(_, v)| v.title().to_string()),
                icon_url: hit.and_then(|(_, v)| v.icon().map(str::to_string)),
                project: hit.map(|(k, _)| k.clone()),
                filename: name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

#[tauri::command]
fn mods_installed(state: State<'_, AppState>) -> Vec<InstalledMod> {
    enrich(&state.store, "mod", state.store.profile_mods("main"), ".jar")
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

    drop_superseded(&state.store, "mod", &project, &file.filename, &state.store.profile_mods("main"));
    // One lookup so the installed list can show a real title and icon later, offline.
    let meta = modrinth::detail(&state.http, &project).await.ok();
    manifest::record(
        &state.store, "main", "mod", &project, &file.filename,
        meta.as_ref().map_or("", |d| d.title.as_str()),
        meta.as_ref().and_then(|d| d.icon_url.clone()),
    )?;
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
    manifest::forget_file(&state.store, "main", "mod", &filename)?;
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
    /// Every jar in the profile's mods folder, not just the Set's members. Fabric loads the
    /// folder, so this is the number that matches what the game reports on startup — a mod
    /// dropped in by hand counts too.
    jars: usize,
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
        jars: launch::count_mods(&dir),
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
    // The Set already knows every member's title and icon — record them so the installed
    // list can show what these jars actually are.
    let mods_dir = state.store.profile_mods("main");
    for m in &r.members {
        drop_superseded(&state.store, "mod", &m.slug, &m.filename, &mods_dir);
        manifest::record(
            &state.store, "main", "mod", &m.slug, &m.filename, &m.title, m.icon_url.clone(),
        )?;
    }
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
    let a = accounts::load(&state.store.root);
    AuthStatus {
        configured: auth::client_id(&state.store.root).is_some(),
        client_id_path: state.store.root.join("client-id.txt").display().to_string(),
        account: a
            .active
            .as_ref()
            .and_then(|id| a.list.iter().find(|x| &x.id == id))
            .map(|x| auth::Account { id: x.id.clone(), name: x.name.clone() }),
    }
}

/* ── accounts ────────────────────────────────────────────────────────────── */

/// Everything the accounts screen needs, in one call.
#[tauri::command]
fn accounts_state(state: State<'_, AppState>) -> accounts::Accounts {
    accounts::load(&state.store.root)
}

#[tauri::command]
fn account_select(state: State<'_, AppState>, id: String) -> Result<accounts::Accounts> {
    accounts::select(&state.store.root, &id)
}

#[tauri::command]
fn account_remove(state: State<'_, AppState>, id: String) -> Result<accounts::Accounts> {
    accounts::remove(&state.store.root, &id)
}

#[tauri::command]
fn offline_name(state: State<'_, AppState>, name: String) -> Result<accounts::Accounts> {
    accounts::set_offline_name(&state.store.root, &name)
}

#[tauri::command]
async fn sign_in(state: State<'_, AppState>) -> Result<auth::Account> {
    let id = auth::client_id(&state.store.root).ok_or_else(|| {
        Error::Other(format!(
            "No Azure client ID yet. Put one in {}",
            state.store.root.join("client-id.txt").display()
        ))
    })?;
    let account = auth::sign_in(&state.http, &id).await?;
    accounts::add(&state.store.root, &account.id, &account.name)?;
    Ok(account)
}


/* ── video defaults ──────────────────────────────────────────────────────── */

/// Force the first-launch video settings onto this profile, and report them as the game's own
/// settings screen words them.
#[tauri::command]
fn video_defaults(state: State<'_, AppState>) -> Result<Vec<String>> {
    let game_dir = state.store.root.join("profiles").join("main");
    let applied = video::apply(&game_dir)?;
    Ok(applied.settings.iter().map(|(k, _)| video::describe(k).to_string()).collect())
}

/* ── the in-game client ──────────────────────────────────────────────────── */

/// Which build of the Vantage client, if any, is in the profile.
#[tauri::command]
fn client_status(state: State<'_, AppState>) -> client::ClientStatus {
    client::status(&state.store)
}

/// Install a client jar from a local path or a URL, replacing any older build.
#[tauri::command]
async fn client_install(state: State<'_, AppState>, source: String) -> Result<String> {
    client::install(&state.http, &state.store, &source).await
}

/* ── launching ───────────────────────────────────────────────────────────── */

#[derive(Serialize)]
pub struct Launched {
    pid: u32,
    java: String,
    classpath_entries: usize,
    loader: String,
    mods: usize,
    offline: bool,
}

/// Start the game. Until Microsoft approves the app this runs an offline session:
/// singleplayer works, online servers correctly refuse it, and the UI says so.
#[tauri::command]
async fn launch_game(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
    memory_mb: u32,
) -> Result<Launched> {
    if state.running.lock().map(|g| g.is_some()).unwrap_or(false) {
        return Err(Error::Other("the game is already running".into()));
    }
    // `name` is what the window asked for; the account store is what the player actually
    // chose. Preferring the store means the launch name cannot drift from the accounts screen.
    let play_as = accounts::play_name(&state.store.root);
    let session = launch::Session::offline(if play_as.is_empty() { &name } else { &play_as });
    let (plan, dir) = launch::plan(&state.http, &state.store, &id, &session, memory_mb).await?;
    let mut child = launch::spawn(&plan, &dir)?;
    let pid = child.id();
    if let Ok(mut g) = state.running.lock() {
        *g = Some(pid);
    }

    // Wait on the child off the async runtime, and tell the window when it goes away, so the
    // Play button reflects reality rather than whatever it last showed.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        if let Some(st) = app2.try_state::<AppState>() {
            if let Ok(mut g) = st.running.lock() {
                *g = None;
            }
        }
        let _ = app2.emit("game:exited", code);
    });

    Ok(Launched {
        pid,
        java: plan.java.clone(),
        classpath_entries: plan.classpath_entries,
        loader: plan.loader.clone(),
        mods: plan.mods,
        offline: true,
    })
}

/// Is the game up right now?
#[tauri::command]
fn game_running(state: State<'_, AppState>) -> Option<u32> {
    state.running.lock().ok().and_then(|g| *g)
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
                running: std::sync::Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            versions, inspect, install, store_info, texture, default_skin, block_list,
            mod_search, mod_install, mods_installed, mod_remove,
            set_status, set_apply, set_remove,
            auth_status, sign_in, launch_game, game_running,
            pack_search, pack_install, packs_installed, pack_remove, installed_ids,
            project_detail, client_status, client_install, video_defaults,
            accounts_state, account_select, account_remove, offline_name
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
        modrinth::search(&http, query, game_version, modrinth::Kind::Mod, 8).await
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
        let mods_dir = store.profile_mods("main");
        for m in &r.members {
            if let Some(old) = manifest::superseded(&store, "main", "mod", &m.slug, &m.filename) {
                let _ = std::fs::remove_file(mods_dir.join(old));
            }
            manifest::record(&store, "main", "mod", &m.slug, &m.filename, &m.title, m.icon_url.clone())?;
        }
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

/// `--video-defaults`: force Vantage's first-launch video settings onto this profile.
///
/// The same settings a new profile gets automatically. Exists because an existing profile is
/// deliberately left alone, and there has to be a way to ask for them anyway.
pub fn headless_video_defaults() {
    let store = store::Store::discover().expect("store");
    let game_dir = store.root.join("profiles").join("main");
    match video::apply(&game_dir) {
        Ok(applied) => {
            println!("applied to {}", game_dir.join("options.txt").display());
            for (k, _) in &applied.settings {
                println!("  {}", video::describe(k));
            }
        }
        Err(e) => {
            eprintln!("could not write options.txt: {e}");
            std::process::exit(1);
        }
    }
}

/// `--client [source]`: report the installed in-game client, or install one.
///
/// With no argument this is a status line. With a path or URL it installs that jar, which is
/// how a locally built client gets into the profile without copying files by hand.
pub fn headless_client(source: Option<&str>) {
    let store = store::Store::discover().expect("store");
    let Some(source) = source else {
        match client::status(&store) {
            ClientStatus { version: Some(v), file: Some(f) } => {
                println!("Vantage client {v}");
                println!("  {}", store.profile_mods("main").join(f).display());
            }
            _ => {
                println!("no Vantage client installed");
                println!("  build one:   cd <client repo> && ./gradlew build");
                println!("  install it:  vantage --client build/libs/vantage-core-<version>.jar");
            }
        }
        return;
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let http = net::client().expect("http client");
    match rt.block_on(client::install(&http, &store, source)) {
        Ok(v) => println!("installed Vantage client {v}"),
        Err(e) => {
            eprintln!("client install failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--launch <version> [name]`: assemble the command line and start the game.
/// Uses an offline session until Microsoft approves the app — singleplayer works,
/// online servers correctly refuse it.
pub fn headless_launch(id: &str, name: Option<&str>) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let outcome = rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        // An explicit name on the command line wins; otherwise play as whoever the accounts
        // screen has chosen, so the two ways in agree about who is playing.
        let who = match name {
            Some(n) => n.to_string(),
            None => accounts::play_name(&store.root),
        };
        let session = launch::Session::offline(&who);
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
    println!("loader      {} ({} mods)", plan.loader, plan.mods);
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
        for line in reader.lines().take(40).map_while(|l| l.ok()) {
            println!("  | {line}");
        }
    }
    println!("(still running — close the game window to exit)");
    let _ = child.wait();
}

/// `--pack <game_version> <kind> <project>`: install a resource pack or shader headlessly
/// and report where the blob and the profile link ended up.
pub fn headless_pack(game_version: &str, kind: &str, project: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let k = match kind {
        "resourcepack" => modrinth::Kind::ResourcePack,
        "shader" => modrinth::Kind::Shader,
        _ => { eprintln!("kind must be resourcepack or shader"); std::process::exit(1); }
    };
    match rt.block_on(async {
        let http = net::client()?;
        let store = store::Store::discover()?;
        let versions = modrinth::versions_of(&http, project, game_version, k).await?;
        let ver = versions.into_iter().next()
            .ok_or_else(|| Error::Other(format!("{project} has no build for {game_version}")))?;
        let file = ver.primary().ok_or_else(|| Error::Other("no file".into()))?.clone();
        let blob = store.pack_blob(&file.hashes.sha1);
        net::fetch_all(&http, vec![net::Item {
            url: file.url.clone(), dest: blob.clone(),
            sha1: Some(file.hashes.sha1.clone()), size: file.size,
        }], Arc::new(net::Counters::default())).await?;
        let dir = store.profile_sub("main", k.profile_dir());
        std::fs::create_dir_all(&dir)?;
        let link = dir.join(&file.filename);
        let _ = std::fs::remove_file(&link);
        let linked = std::fs::hard_link(&blob, &link).is_ok();
        if !linked { std::fs::copy(&blob, &link)?; }
        if let Some(old) = manifest::superseded(&store, "main", kind, project, &file.filename) {
            let _ = std::fs::remove_file(dir.join(old));
        }
        let meta = modrinth::detail(&http, project).await.ok();
        manifest::record(
            &store, "main", kind, project, &file.filename,
            meta.as_ref().map_or("", |d| d.title.as_str()),
            meta.as_ref().and_then(|d| d.icon_url.clone()),
        )?;
        Ok::<_, Error>((ver.version_number, file, blob, link, linked))
    }) {
        Ok((num, file, blob, link, linked)) => {
            println!("{project} {num} — {} ({:.2} MB)", file.filename, file.size as f64 / 1048576.0);
            println!("  blob {}", blob.display());
            println!("  link {}  [{}]", link.display(), if linked { "hard link" } else { "copy" });
        }
        Err(e) => { eprintln!("pack install failed: {e}"); std::process::exit(1); }
    }
}
