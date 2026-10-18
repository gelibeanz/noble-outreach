#!/bin/bash
# Build and run reach-host under Proton via umu (GE-Proton under compatibilitytools.d).
# Extra args go to reach-host (--options <file> | --map <name|id> [--game-variant <.bin>] [--map-variant <.mvar>], --no-launch). Log: next to the exe, reach-host.log.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
EXE="$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
cd "$ROOT"
exec umu-run "$EXE" --mcc "$MCC_WIN" "$@"
