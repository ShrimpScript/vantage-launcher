import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Picker, type PickerItem } from "./picker";

type Entry = { id: string; type: string; url: string; sha1: string; releaseTime: string };
type Versions = {
  latest: { release: string; snapshot: string };
  releases: Entry[]; snapshots: Entry[]; total: number;
};
type Inspection = {
  id: string; main_class: string;
  java: { component: string; majorVersion: number } | null;
  asset_index_id: string; asset_objects: number; asset_bytes: number;
  client_bytes: number; libs_total: number; libs_applicable: number;
  os: string; installed: boolean;
};
type Progress = { phase: string; done: number; total: number; bytes: number; total_bytes: number; skipped: number };
type Report = {
  id: string; libs_applicable: number; libs_total: number; files: number;
  downloaded: number; skipped: number; bytes: number; seconds: number; store_root: string;
};
type Installed = { id: string; jar_bytes: number };
type Hit = {
  project_id: string; slug: string; title: string; description: string;
  author: string; downloads: number; icon_url: string | null; categories: string[];
};
type InstalledMod = { filename: string; bytes: number };
type SetMember = {
  slug: string; title: string; role: string; version_number: string;
  version_type: string; filename: string; bytes: number; installed: boolean;
  icon_url: string | null; color: number | null;
};
type SetView = {
  members: SetMember[]; total_bytes: number; applied: boolean;
  loader: string; version_id: string;
};
type SetReport = { installed: number; bytes: number; mrpack: string; seconds: number };
type Account = { id: string; name: string };
type AuthStatus = { configured: boolean; client_id_path: string; account: Account | null };
type Launched = { pid: number; java: string; classpath_entries: number; offline: boolean };
type ModInstalled = { filename: string; version_number: string; version_type: string; bytes: number };
type StoreInfo = { root: string; files: number; bytes: number; versions: Installed[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const gb = (b: number) => (b >= 1073741824 ? `${(b / 1073741824).toFixed(2)} GB` : `${(b / 1048576).toFixed(0)} MB`);
const n = (v: number) => v.toLocaleString();

const go = $<HTMLButtonElement>("go");
const selected = { value: "" };
let mode: "install" | "verify" | "play" = "install";

function say(html: string, kind: "" | "ok" | "bad" = "") {
  const m = $("msg");
  m.className = `msg ${kind}`.trim();
  m.innerHTML = html;
  m.hidden = false;
}

/** The launch card's thumbnail is a real block texture from the selected version's jar. */
async function setArt(id: string, installed: boolean) {
  const img = $<HTMLImageElement>("verart");
  const fb = $("verfallback");
  fb.textContent = id.split(".")[0] ?? "MC";
  if (!installed) { img.hidden = true; fb.hidden = false; return; }
  try {
    const uri = await invoke<string>("texture", { id, name: "grass_block_side" });
    img.src = uri;
    img.hidden = false;
    fb.hidden = true;
  } catch {
    img.hidden = true;
    fb.hidden = false;
  }
}

/* ── the account face: the ONE place a player appears (DESIGN.md §7) ──────
   Cropped from the 8x8 head region of the skin in the selected version's jar,
   scaled with pixelated rendering. No 3D, no model, no second appearance. */
async function setFace(id: string, installed: boolean) {
  const img = $<HTMLImageElement>("face");
  if (!installed) { img.hidden = true; return; }
  try {
    const skin = await invoke<string>("default_skin", { id });
    const src = new Image();
    src.src = skin;
    await src.decode();
    const c = document.createElement("canvas");
    c.width = c.height = 8;
    const ctx = c.getContext("2d")!;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(src, 8, 8, 8, 8, 0, 0, 8, 8);      // face
    ctx.drawImage(src, 40, 8, 8, 8, 0, 0, 8, 8);     // hat layer over it
    img.src = c.toDataURL();
    img.hidden = false;
  } catch {
    img.hidden = true;
  }
}

function renderCards(store: StoreInfo) {
  const cards = $("cards");
  const frag = document.createDocumentFragment();

  for (const v of store.versions) {
    const el = document.createElement("button");
    el.className = "card";
    el.type = "button";
    el.innerHTML =
      `<span class="id">${v.id}</span>` +
      `<span class="meta">Client ${gb(v.jar_bytes)}</span>` +
      `<span class="tag">Installed</span>`;
    el.addEventListener("click", () => { selected.value = v.id; picker.select(v.id); void inspect(v.id); });
    frag.append(el);
  }

  const add = document.createElement("button");
  add.className = "card add";
  add.type = "button";
  add.innerHTML = `<span class="id">Add a version</span><span class="meta">Pick from 900+ releases and snapshots</span>`;
  add.addEventListener("click", () => $("version").click());
  frag.append(add);

  cards.replaceChildren(frag);
  $("statbytes").textContent = store.files ? gb(store.bytes) : "Empty";
  $("statfiles").textContent = store.files ? `${n(store.files)} files` : "nothing installed";
  $("statfiles").title = store.root;
}

async function refreshStore() {
  renderCards(await invoke<StoreInfo>("store_info"));
}

let token = 0;
async function inspect(id: string) {
  const mine = ++token;
  $("state").textContent = "Reading the manifest…";
  go.disabled = true;
  try {
    const i = await invoke<Inspection>("inspect", { id });
    if (mine !== token) return;

    $("pctitle").textContent = `Minecraft ${i.id}`;
    if (i.installed) {
      go.textContent = "Play";
      go.disabled = false;
      mode = "play";
      $("pcmeta").textContent = "Ready to play";
      $("state").textContent = "Offline session — singleplayer only until you sign in.";
    } else {
      go.textContent = "Install";
      go.disabled = false;
      mode = "install";
      $("pcmeta").textContent = `${gb(i.client_bytes + i.asset_bytes)} to download`;
      $("state").textContent = "";
    }
    await setArt(id, i.installed);
    await setFace(id, i.installed);
    void refreshSet();
  } catch (e) {
    if (mine !== token) return;
    $("state").textContent = "";
    say(`Could not read that version: ${e}`, "bad");
  }
}

const picker = new Picker($<HTMLButtonElement>("version"), $("vlabel"), [], selected);
picker.onChange((id) => void inspect(id));

async function boot() {
  try {
    const v = await invoke<Versions>("versions");
    const rows: PickerItem[] = [
      ...v.releases.map((e) => ({ id: e.id, group: "Releases", meta: e.releaseTime.slice(0, 10) })),
      ...v.snapshots.map((e) => ({ id: e.id, group: "Snapshots", meta: e.releaseTime.slice(0, 10) })),
    ];
    picker.setItems(rows, v.latest.release);
    await inspect(v.latest.release);
  } catch (e) {
    say(`Could not reach Mojang&rsquo;s version manifest: ${e}`, "bad");
  }
  await refreshStore();
}

listen<Progress>("install:progress", ({ payload: p }) => {
  $("bar").hidden = false;
  $("barfill").style.width = `${(p.total ? (p.done / p.total) * 100 : 0).toFixed(2)}%`;
  const label = p.phase === "core" ? "Client and libraries" : "Assets";
  $("barmeta").textContent =
    `${label} · ${n(p.done)} / ${n(p.total)} files · ${(p.bytes / 1048576).toFixed(0)} of ${(p.total_bytes / 1048576).toFixed(0)} MB`;
});

async function runInstall(id: string, verifying: boolean) {
  go.disabled = true;
  $("state").textContent = verifying ? `Verifying ${id}…` : `Installing ${id}…`;
  try {
    const r = await invoke<Report>("install", { id });
    const rate = (r.files / Math.max(r.seconds, 0.001)).toFixed(0);
    say(
      r.downloaded === 0
        ? `<b>${r.id} verified.</b> All ${n(r.files)} files matched their published hashes in <b>${r.seconds.toFixed(1)}s</b> — nothing needed re-downloading.`
        : `<b>${r.id} is ready.</b> ${n(r.files)} files verified in <b>${r.seconds.toFixed(1)}s</b> — ${rate} files a second, ${gb(r.bytes)} over the wire.`,
      "ok",
    );
    $("barfill").style.width = "100%";
  } catch (e) {
    say(`Install failed: ${e}`, "bad");
  } finally {
    $("bar").hidden = true;
    await refreshStore();
    await inspect(id);
  }
}

go.addEventListener("click", async () => {
  const id = selected.value;
  if (mode !== "play") {
    await runInstall(id, mode === "verify");
    return;
  }
  go.disabled = true;
  go.textContent = "Starting…";
  try {
    await invoke<Launched>("launch_game", { id, name: "Player", memoryMb: 4096 });
    say(`<b>Minecraft ${id} is running.</b>`, "ok");
  } catch (e) {
    say(`Could not start the game: ${e}`, "bad");
  } finally {
    go.disabled = false;
    go.textContent = `Play ${id}`;
  }
});




/* ── screen switching ────────────────────────────────────────────────────── */
const screens = document.querySelectorAll<HTMLElement>(".screen");
const railBtns = document.querySelectorAll<HTMLButtonElement>(".rail button[data-goto]");
function goto(name: string) {
  screens.forEach((s) => (s.hidden = s.dataset.screen !== name));
  railBtns.forEach((b) => {
    const on = b.dataset.goto === name;
    b.classList.toggle("on", on);
    if (on) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  });
  if (name === "mods") {
    void refreshMods();
    // a search screen that needs a click before you can type is a search screen with a bug
    requestAnimationFrame(() => q.focus());
  }
  if (name === "settings") void refreshSettings();
}
railBtns.forEach((b) => b.addEventListener("click", () => goto(b.dataset.goto!)));

// Keyboard is the primary path (DESIGN.md §7): every screen is one chord away.
const ORDER = ["play", "mods", "settings"];
window.addEventListener("keydown", (e) => {
  if (!e.ctrlKey && !e.metaKey) return;
  const i = Number(e.key) - 1;
  if (Number.isInteger(i) && i >= 0 && i < ORDER.length) {
    e.preventDefault();
    goto(ORDER[i]!);
  }
});

/* ── mods: real Modrinth search against the selected game version ────────── */
const q = $<HTMLInputElement>("q");
let searchToken = 0;
let searchTimer: number | undefined;

function modRow(h: Hit, installedNames: Set<string>): HTMLElement {
  const row = document.createElement("div");
  row.className = "row";
  const icon = h.icon_url
    ? `<img class="ic" src="${h.icon_url}" alt="" loading="lazy" />`
    : `<div class="ic"></div>`;
  row.innerHTML =
    icon +
    `<div class="body">` +
    `<span class="t">${h.title}</span>` +
    `<span class="d">${h.description}</span>` +
    `<span class="by">${h.author} · <span class="dl">${h.downloads.toLocaleString()}</span> downloads</span>` +
    `</div>`;
  const btn = document.createElement("button");
  const already = [...installedNames].some((f) => f.toLowerCase().includes(h.slug.replace(/-/g, "")));
  btn.className = already ? "act" : "act primary";
  btn.textContent = already ? "Added" : "Add";
  btn.disabled = already;
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "Adding…";
    try {
      const r = await invoke<ModInstalled>("mod_install", { project: h.project_id, gameVersion: selected.value });
      btn.className = "act";
      btn.textContent = "Added";
      const kind = r.version_type === "release" ? "" : ` (${r.version_type})`;
      modSay(`<b>${h.title} ${r.version_number}${kind}</b> installed — ${(r.bytes / 1048576).toFixed(1)} MB, sha1 verified, straight from cdn.modrinth.com.`, "ok");
      await refreshMods();
    } catch (e) {
      btn.disabled = false;
      btn.textContent = "Add";
      modSay(`Could not add ${h.title}: ${e}`, "bad");
    }
  });
  row.append(btn);
  return row;
}

function modSay(html: string, kind: "" | "ok" | "bad" = "") {
  const m = $("modmsg");
  m.className = `msg ${kind}`.trim();
  m.innerHTML = html;
  m.hidden = false;
}

async function runSearch(term: string) {
  const mine = ++searchToken;
  const out = $("results");
  if (!term.trim()) { out.replaceChildren(); return; }
  // skeletons, never a spinner
  out.replaceChildren(...Array.from({ length: 4 }, () => {
    const s = document.createElement("div"); s.className = "skel"; return s;
  }));
  try {
    const [hits, installed] = await Promise.all([
      invoke<Hit[]>("mod_search", { query: term, gameVersion: selected.value }),
      invoke<InstalledMod[]>("mods_installed"),
    ]);
    if (mine !== searchToken) return;
    const names = new Set(installed.map((m) => m.filename.toLowerCase()));
    if (!hits.length) {
      out.replaceChildren(Object.assign(document.createElement("p"), {
        className: "empty",
        textContent: `Nothing on Modrinth matches “${term}” for Fabric ${selected.value}. Try a different version, or drop a .jar in the mods folder.`,
      }));
      return;
    }
    out.replaceChildren(...hits.map((h) => modRow(h, names)));
  } catch (e) {
    if (mine !== searchToken) return;
    out.replaceChildren();
    modSay(`Modrinth search failed: ${e}`, "bad");
  }
}

q.addEventListener("input", () => {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => void runSearch(q.value), 220);
});

async function refreshMods() {
  $("modctx").textContent = `Fabric · ${selected.value}`;
  const installed = await invoke<InstalledMod[]>("mods_installed");
  const list = $("installed");
  $("modtot").textContent = installed.length
    ? `${installed.length} · ${(installed.reduce((a, m) => a + m.bytes, 0) / 1048576).toFixed(1)} MB`
    : "";
  if (!installed.length) {
    list.replaceChildren(Object.assign(document.createElement("p"), {
      className: "empty",
      textContent: "No mods yet. Search above and hit Add — files come from Modrinth unmodified, verified against their published hash.",
    }));
    return;
  }
  list.replaceChildren(...installed.map((m) => {
    const row = document.createElement("div");
    row.className = "row";
    row.innerHTML =
      `<div class="ic"></div><div class="body"><span class="t">${m.filename}</span>` +
      `<span class="by"><span class="dl">${(m.bytes / 1048576).toFixed(1)}</span> MB</span></div>`;
    const rm = document.createElement("button");
    rm.className = "act quiet";
    rm.textContent = "Remove";
    rm.addEventListener("click", async () => {
      try { await invoke("mod_remove", { filename: m.filename }); await refreshMods(); }
      catch (e) { modSay(`Could not remove: ${e}`, "bad"); }
    });
    row.append(rm);
    return row;
  }));
}

/* ── settings ────────────────────────────────────────────────────────────── */
async function refreshSettings() {
  const s = await invoke<StoreInfo>("store_info");
  $("storepath").textContent = s.root;
  $("storeval").textContent = s.files ? `${gb(s.bytes)} · ${n(s.files)} files` : "empty";

  const a = await invoke<AuthStatus>("auth_status");
  const btn = $<HTMLButtonElement>("signin");
  btn.disabled = !a.configured;
  $("authhint").innerHTML = a.configured
    ? `Ready. Sign-in opens your browser once and comes straight back — no code to type. Refresh tokens go to your OS keychain, never a file.`
    : `Needs an Azure client ID (public, not a secret). Register an app for personal Microsoft accounts, apply at <code>https://aka.ms/mce-reviewappid</code>, then put the ID in <code>${a.client_id_path}</code>.`;
}
$("signin").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("signin");
  btn.disabled = true;
  btn.textContent = "Waiting for your browser…";
  try {
    const acct = await invoke<Account>("sign_in");
    document.querySelector(".who")!.textContent = acct.name;
    document.querySelector("#acctsub")!.textContent = "Signed in";
    btn.textContent = "Signed in";
  } catch (e) {
    btn.disabled = false;
    btn.textContent = "Sign in";
    $("authhint").textContent = String(e);
  }
});

const rm = $<HTMLButtonElement>("rm");
rm.addEventListener("click", () => {
  const on = rm.getAttribute("aria-checked") !== "true";
  rm.setAttribute("aria-checked", String(on));
  document.documentElement.classList.toggle("calm", on);
});


/* ── the Vantage Set ─────────────────────────────────────────────────────
   Bundled by default, openable all the way down. Resolved live from Modrinth and
   pinned by hash; the same data is exported as a real .mrpack so nobody is locked in. */


async function refreshSet() {
  const grid = $("setrows");
  const btn = $<HTMLButtonElement>("setbtn");
  grid.replaceChildren(...Array.from({ length: 5 }, () => {
    const s = document.createElement("div");
    s.className = "skel";
    s.style.height = "116px";
    return s;
  }));
  try {
    const v = await invoke<SetView>("set_status", { gameVersion: selected.value });
    grid.replaceChildren(
      ...v.members.map((m) => {
        const el = document.createElement("button");
        el.className = "modcard";
        el.type = "button";
        el.title = `${m.title} ${m.version_number}`;
        const icon = m.icon_url
          ? `<img class="mi" src="${m.icon_url}" alt="" loading="lazy" />`
          : `<div class="mi"></div>`;
        el.innerHTML =
          `<span class="dot${m.installed ? "" : " off"}"></span>` + icon +
          `<span class="mn">${m.title}</span><span class="mr">${m.role}</span>`;
        el.addEventListener("click", () => goto("mods"));
        return el;
      }),
    );
    const on = v.members.filter((m) => m.installed).length;
    if (mode === "play") {
      $("pcmeta").textContent = `Fabric ${v.loader} · ${on} mod${on === 1 ? "" : "s"}`;
    }
    $("settot").textContent = v.applied ? "Applied" : `${v.members.length} pinned`;
    $("setsub").textContent = `${(v.total_bytes / 1048576).toFixed(1)} MB · Fabric ${v.loader}`;
    btn.hidden = false;
    btn.textContent = v.applied ? "Remove" : "Apply";
    btn.className = v.applied ? "act quiet wide" : "act primary wide";

  } catch (e) {
    grid.replaceChildren();
    $("setsub").textContent = `Could not resolve: ${e}`;
  }
}

$("setbtn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("setbtn");
  const removing = btn.textContent === "Remove";
  btn.disabled = true;
  btn.textContent = removing ? "Removing…" : "Applying…";
  try {
    if (removing) {
      const n2 = await invoke<number>("set_remove", { gameVersion: selected.value });
      say(`Removed ${n2} mods. The Set is a toggle, not a cage.`, "ok");
    } else {
      const r = await invoke<SetReport>("set_apply", { gameVersion: selected.value });
      say(
        `<b>The Vantage Set is on.</b> ${r.installed} mods, ${(r.bytes / 1048576).toFixed(1)} MB, ` +
          `verified in ${r.seconds.toFixed(1)}s — and written out as a real .mrpack you can audit or take elsewhere.`,
        "ok",
      );
    }
  } catch (e) {
    say(`${removing ? "Remove" : "Apply"} failed: ${e}`, "bad");
  } finally {
    await refreshSet();
    await refreshStore();
  }
});

void boot();
