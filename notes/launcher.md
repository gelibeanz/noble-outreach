# Launcher (reach-launcher)

`src/reach-launcher`: what friends download instead of a zip. eframe/egui window in the menu's colours (copied
from reach-host `ui/theme.rs`). One file per OS: `dist/NobleOutreach-Launcher.exe` (x86_64-pc-windows-gnu, wgpu
renderer) and `dist/NobleOutreach-Launcher-linux` (glibc 2.31+, OpenGL via glow, X11 + Wayland).
Build both + stage client files: `scripts/make-launcher.sh` (NO_HOST=1 skips rebuilding reach-host).
Friend text: `notes/launcher-README.txt` (copied to dist/README.txt).

## What it does
1. **MCC folder**: saved choice, else Steam libraries: roots (Windows `reg query HKCU\Software\Valve\Steam /v SteamPath`,
   `C:\Program Files (x86)\Steam`; Linux `~/.steam/steam`, `~/.local/share/Steam`, `~/.steam/root`, Flatpak
   `~/.var/app/com.valvesoftware.Steam/{.local/share,data}/Steam`, snap) + every `"path"` in
   `steamapps/libraryfolders.vdf`, then `steamapps/common/Halo The Master Chief Collection` with
   `haloreach/haloreach.dll`. Browse... (rfd; Linux uses the xdg portal) or type a path.
2. **Build check**: `haloreach/maps`, `haloreach/haloreach.dll`, `MCC/Binaries/Win64/bink2w64.dll` and
   `xaudio2_9redist.dll` must exist; sha256 of haloreach.dll must be the frozen one (baked in `check.rs`, same as
   work/mcc/SHA256-frozen.txt). Mismatch: "Your MCC build is <build_tag.txt>; Noble Outreach needs build
   2025.08.16.178512.1", PLAY stays disabled. A new MCC build means a new FROZEN_SHA256 + re-checked addresses.
3. **Client update** (`update::resolve`): the source is built in with `NOBLE_UPDATE_SOURCE`:
   - `github:<owner>/<repo>` (default `github:gelibeanz/noble-outreach`): `GET api.github.com/repos/<repo>/releases/latest`
     (User-Agent set), then the release's `SHA256SUMS` asset; files = `reach-host.exe`, launcher builds =
     `NobleOutreach-Launcher.exe` / `-linux`, each only if listed in SHA256SUMS; version = the tag. 404 = "no release
     published yet", 403/429 = rate limit (60 unauthenticated requests per hour per IP).
   - `master:<url>[,<url>...]`: first master whose `/healthz` says ok, its `/api/client/manifest`.
   Run time: `NOBLE_UPDATE_SOURCE` replaces the built-in source, `REACH_LAUNCHER_MASTERS=a,b` forces master mode.
   Then download each file whose local size/sha256 differs to `<name>.part`, verify sha256 + size, rename.
   `client-version.txt` = last good version. If no source answers, the message says so and PLAY still works with
   the installed client.
4. **Play**: writes `player_name` + `master_url` (the master that answered; not touched with the GitHub source) into
   `<client>/reach-data/settings.json` (merged, other menu settings kept), sets `REACH_NAME`, starts
   `reach-host.exe --mcc <dir>` with cwd = client dir, stdout/stderr -> `<data>/launch.log`.
   Windows: direct, CREATE_NO_WINDOW. Linux: `umu-run` with `WINEPREFIX=<data>/prefix` (an inherited WINEPREFIX
   wins), `GAMEID=umu-976730`, `PROTONPATH=<chosen Proton>` or `GE-Proton` (umu downloads the latest), MCC path as
   `Z:\...`. No `--master` arg: in menu mode reach-host takes the master from settings.json, and a CLI `--master`
   would make every later local host game register.
   Status: last 6 lines of `<client>/reach-host.log`, "Open logs folder" opens the client dir.
5. **Self-update** (`update::self_update`, runs first in the same start-up job, before the client sync): the manifest's
   `launcher.version` differs from this build's `REACH_LAUNCHER_BUILD` (build.rs: env `REACH_LAUNCHER_VERSION`, else
   date + short commit) -> download the OS's entry to `<exe>.new`, verify sha256 + size, then replace.
   Linux: chmod 755, rename over the exe (atomic), `exec` with the same args. Windows: rename the running exe to
   `<exe>.old`, rename `.new` into place (restores `.old` if that fails), spawn the new exe with the same args, exit;
   `update::cleanup_old` deletes `.old` at the next start. The relaunched process gets
   `REACH_LAUNCHER_NO_SELFUPDATE=1`, so it never updates twice (no loop). Any failure keeps the current launcher
   running and shows red "Launcher update failed: <reason>" (client sync still runs); the header shows
   "Updating launcher to X..." while downloading. Same `?sha=` URLs as client files. The exe folder must be writable.
6. Settings: player name (max 15 chars), MCC folder; Linux: Proton list (`compatibilitytools.d/*` with a `proton`
   file in every Steam root, `steamapps/common/Proton*` in every library, GE first) + a umu-run install hint
   when it is missing (`PATH`, `~/.local/bin`).

Data dir: Windows `%LOCALAPPDATA%\NobleOutreach`, Linux `~/.local/share/NobleOutreach` (`REACH_LAUNCHER_DATA`
overrides): `launcher.json` (mcc_dir, player_name, proton), `client/`, `prefix/`, `launch.log`.
Rename migration (`migrate_data_dir`, first thing in `main`): when `NobleOutreach` does not exist and the old
`ReachReclaimer` does, rename it (settings, client, prefix move together); if the rename fails, copy
`launcher.json` and the client files and let the prefix be set up again. Skipped when `REACH_LAUNCHER_DATA` is set.
Test hook: `REACH_LAUNCHER_AUTOPLAY=1` presses PLAY once when it becomes enabled.

Windows build under Wine/Proton: wgpu's default D3D12 (vkd3d) shows a white window; the launcher sets
`WGPU_BACKEND=vulkan` when any `WINE*` env var is present (Wine passes the Unix env through). The glow (OpenGL)
renderer was white under Wine too, so Windows uses wgpu.

## Master side (`src/reach-master/src/client.rs`)
- `--client-dir DIR` (env `REACH_MASTER_CLIENT_DIR`). Without it the client routes answer 404.
- `GET /api/client/manifest` -> `{version, files: [{name, sha256, size}]}`. Every regular file in DIR with a plain
  name (`[A-Za-z0-9._-]`, no leading dot) except `version.txt`; version = first line of `version.txt`, else 12 hex of a
  hash over the entries. Hashes cached per (size, mtime), so replacing files needs no restart.
- `GET /api/client/files/<name>` -> the file, streamed (32 MB container limit), only names the manifest would list.
- Launcher builds: `<DIR>/launcher/{NobleOutreach-Launcher.exe,NobleOutreach-Launcher-linux,launcher-version.txt}`
  (staged by make-launcher.sh); the old `ReachReclaimer-Launcher*` names are served when a new one is missing.
  The self-update replaces the running exe's own path whatever the manifest calls the file, so a launcher from
  before the rename updates to the renamed build (it keeps its old file name on disk). Manifest gets `launcher: {version, windows: {name, sha256, size}, linux: {...}}`
  (absent when `launcher-version.txt` is missing; each OS key absent when its file is). Served via the same
  `/api/client/files/<name>` route (only the launcher names, from `launcher/`).
- Deployment: mount the staged `dist/client` read-only and pass `--client-dir`; upload with rsync
  `--delay-updates`, so no half-written exe is served (the private deploy scripts do this).

## Known limits
- No update source reachable: PLAY uses the installed client; a first install needs the source.
- Self-update needs a writable exe folder; launchers older than this feature (no self-update) need one more manual download.
- Windows exe unsigned (SmartScreen "More info > Run anyway").
- Linux binary needs X11 or Wayland + an OpenGL driver (glow); built against glibc 2.31.
