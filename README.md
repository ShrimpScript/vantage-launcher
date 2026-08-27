# Vantage

A Minecraft: Java Edition launcher for Windows, macOS and Linux. Free, no advertising, no
telemetry.

Built with Tauri v2 and Rust. The release binary is **4.9 MB** and the frontend is 78 KB
including bundled fonts, because a launcher should not cost more than the thing it launches.

> **Early.** Version installation, mod management, the mod bundle and launching the game all
> work today. **Signing in does not yet** — that needs a Microsoft API permission which is
> pending approval, so online play is unavailable and the UI says so rather than pretending
> otherwise. Singleplayer runs on an offline session in the meantime.

## What works

- **Version installation.** Reads Mojang's live version manifest and installs any release or
  snapshot. A full 26.2 install is 5,127 files and 572 MB, verified in **21 seconds** on a
  desktop connection — 20 parallel connections, every file checked against its published sha1,
  written temp-then-rename so an interrupted download never leaves a truncated file.
- **A content-addressed store** in a vanilla-compatible layout, so other launchers can read it
  and you can leave with your install intact. Re-installing a version you already have skips
  everything that is already present.
- **Modrinth integration.** Search and install mods for the selected version. Files come from
  each author's official release, unmodified, verified against the published hash.
- **The Vantage Set** — a pinned performance stack (Sodium, Lithium, ImmediatelyFast,
  FerriteCore, Fabric API) that installs in one click and exports as a real `.mrpack` you can
  audit or import into another launcher. It can be removed just as easily; a bundle you cannot
  switch off is not a convenience.
- **Launching.** Provisions the exact Java runtime the version asks for (26.2 wants Java 25),
  assembles the classpath, resolves Mojang's rule-gated argument templates and starts the game.
  The Java component manifest is 434 entries on Linux — 147 files, 82 directories and **205
  symlinks** — and all three are handled, because missing the links leaves a subtly broken JRE.
- **Microsoft sign-in** via OAuth 2.0 authorization code + PKCE over a loopback redirect. No
  code to type, no second device. Refresh tokens go to the OS credential store.

## What it does not do

- No advertising surface, and no surface that refreshes on a timer. This is enforced in the
  codebase, not in a policy document.
- No telemetry. Nothing is collected. If that ever changes it will arrive as an opt-in screen
  listing every field by name, defaulted off.
- No feature is gated behind payment.

## Building

Requires Rust, Node 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
for your platform.

```bash
npm install
npm run tauri dev      # development
npm run tauri build    # release
```

There is also a headless mode, so the download pipeline can be exercised without a window and
the GUI and CI paths cannot drift apart:

```bash
vantage-launcher --install 26.2            # install a version
vantage-launcher --mods 26.2 sodium        # search Modrinth
vantage-launcher --add 26.2 lithium        # install one mod
vantage-launcher --set 26.2                # apply the Vantage Set and write the .mrpack
vantage-launcher --launch 26.2 Player       # assemble the command line and start the game
vantage-launcher --auth-status             # is a Microsoft client ID configured
```

### Signing in

Sign-in needs an Azure application client ID (public — this is a PKCE public client, there is
no secret). Register an app for **personal Microsoft accounts only**, add `http://127.0.0.1` as
a **Mobile and desktop** redirect URI, then apply for Minecraft API permission at
<https://aka.ms/mce-reviewappid>. Put the ID in `client-id.txt` in the launcher's data
directory, or set `VANTAGE_CLIENT_ID`.

Until the application is approved, `api.minecraftservices.com` answers 403 regardless of how
correct the tokens are.

### A note for Linux users on NVIDIA

WebKitGTK's DMABUF renderer crashes with a Wayland protocol error on NVIDIA drivers. Vantage
sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` itself before GTK initialises, unless you have already
set it, so you should not have to think about it.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE).

Vantage is not affiliated with Mojang Studios or Microsoft. Sodium, Lithium, ImmediatelyFast,
FerriteCore and Fabric are the work of their respective authors and are downloaded unmodified
from their official releases under their own licences.
