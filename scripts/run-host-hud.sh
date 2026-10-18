#!/bin/bash
# Like run-host.sh but isolated for HUD/text work: own prefix (work/prefix-hud) and exe copy (work/run-hud).
# Play: scripts/run-host-hud.sh --map forge_world   (click the window to capture the mouse, F10 toggles, Esc frees it)
# Scripted check: NO_BUILD=1 scripts/run-host-hud.sh --map forge_world --test-input scripts/test-input-move.txt
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-hud"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
mkdir -p "$ROOT/work/run-hud"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
if [ -z "$NO_COPY" ]; then cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-hud/reach-host.exe"; fi
cd "$ROOT"
exec umu-run "$ROOT/work/run-hud/reach-host.exe" --mcc "$MCC_WIN" "$@"
