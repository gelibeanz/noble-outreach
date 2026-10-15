# Noble Outreach 0.1.0

First public release of Noble Outreach, a standalone community client for Halo: Reach on PC. It runs the
engine from your own Steam copy of Halo: The Master Chief Collection.

**You need:** MCC on Steam, build **2025.08.16** (`2025.08.16.178512.1`), with Halo: Reach installed. Linux
also needs [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher).

## Downloads
- `NobleOutreach-Launcher.exe`: Windows launcher. It finds MCC, checks the build, installs and updates the
  game client from this repository's releases, and starts it. The exe is unsigned, so SmartScreen may warn:
  More info > Run anyway.
- `NobleOutreach-Launcher-linux`: Linux launcher (x86_64, glibc 2.31+, X11 or Wayland). `chmod +x` it, then
  run it. The game runs through umu + GE-Proton.
- `reach-host.exe`: the game client the launchers install. You don't need to download it yourself.
- `NobleOutreach-Server-<version>.tar.gz`: for hosting. `reach-host.exe` runs a dedicated server (`--dedicated`,
  or the Docker setup in `docker/`). `reach-master` is a static Linux build of the master server.
- `SHA256SUMS`: the launcher checks every download against it.

## What's in it
- Host your own games: listen servers from the menu (UPnP; forward UDP 47800 if needed) and dedicated servers
  (Windows or Docker), with playlist voting.
- Server browser: Direct Connect by address, favourites, LAN games, and any master server you add in Settings.
  No master is built in. `reach-master` is included if you want to run one.
- Multiplayer custom games, Firefight, and online campaign co-op (4 players) with skulls.
- Forge: solo, on a listen server or on a dedicated server. Save your map variants and play them.
- Spartan/Elite customisation with a live 3D preview. Settings, key bindings, scoreboard, and a pause overlay.

## Known limits
- Works with one exact MCC build only. When Steam updates MCC, wait for a matching release.
- A dedicated server in Docker needs an NVIDIA GPU. AMD and Intel GPUs are untested.
- The launcher checks GitHub without signing in (60 checks per hour per IP). If GitHub can't be reached, the
  installed client keeps working.

Not affiliated with Microsoft, 343 Industries or Halo Studios. No game files are included. GPL-3.0.
