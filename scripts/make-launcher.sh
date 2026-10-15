#!/usr/bin/env bash
# Build the friend launcher for both OSes and stage the client files the master serves.
#   dist/NobleOutreach-Launcher.exe     Windows (x86_64-pc-windows-gnu)
#   dist/NobleOutreach-Launcher-linux   Linux x86_64 (glibc 2.31+, i.e. Ubuntu 20.04 / Debian 11 and newer)
#   dist/client/                         reach-host.exe + version.txt -> reach-master --client-dir
#   dist/client/launcher/                both launchers + launcher-version.txt (self-update, listed as `launcher` in the manifest)
#   dist/README.txt                      short friend instructions
# Env: CARGO_TARGET_DIR (default target/), NO_HOST=1 skips rebuilding reach-host (uses the last build),
#      NOBLE_DEFAULT_MASTERS=url[,url...] default master list baked into reach-host (default: none),
#      NOBLE_UPDATE_SOURCE=master:url[,url...] | github:owner/repo  where the launcher updates from (default:
#      github:gelibeanz/noble-outreach; dist/client is for a master source). scripts/private-env.sh sets both.
# Upload the client files: scripts/deploy-client.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tgt="${CARGO_TARGET_DIR:-$root/target}"
dist="$root/dist"
# shellcheck disable=SC1091
[ -f "$root/scripts/private-env.sh" ] && . "$root/scripts/private-env.sh"
mkdir -p "$dist/client"
cd "$root"
# Launcher build version (date + short commit [+dirty]): embedded in both launchers (build.rs) and written to
# dist/client/launcher/launcher-version.txt, which the master puts in its manifest so older launchers update themselves.
lver="$(date +%Y.%m.%d)-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
git diff --quiet 2>/dev/null || lver="$lver-dirty"
export REACH_LAUNCHER_VERSION="$lver"
mkdir -p "$dist/client/launcher"
# Launcher builds from before the rename (the master falls back to them when the new names are missing).
rm -f "$dist/client/launcher/ReachReclaimer-Launcher.exe" "$dist/client/launcher/ReachReclaimer-Launcher-linux"
cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-launcher
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31 -p reach-launcher
[ -n "${NO_HOST:-}" ] || cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host
cp "$tgt/x86_64-pc-windows-gnu/release/reach-launcher.exe" "$dist/NobleOutreach-Launcher.exe"
cp "$tgt/x86_64-unknown-linux-gnu/release/reach-launcher" "$dist/NobleOutreach-Launcher-linux"
chmod +x "$dist/NobleOutreach-Launcher-linux"
# Same files again under client/launcher/ for the self-update (the name is what the master's manifest lists).
cp "$dist/NobleOutreach-Launcher.exe" "$dist/NobleOutreach-Launcher-linux" "$dist/client/launcher/"
echo "$lver" > "$dist/client/launcher/launcher-version.txt"
cp "$tgt/x86_64-pc-windows-gnu/release/reach-host.exe" "$dist/client/reach-host.exe"
# Client version: date + short commit (+dirty), shown in the launcher.
ver="$(date +%Y.%m.%d)-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
git diff --quiet 2>/dev/null || ver="$ver-dirty"
echo "$ver" > "$dist/client/version.txt"
cp "$root/notes/launcher-README.txt" "$dist/README.txt"
(cd "$dist" && sha256sum NobleOutreach-Launcher.exe NobleOutreach-Launcher-linux client/reach-host.exe client/launcher/*)
echo "launcher version $lver, client version $ver staged in $dist/client"
