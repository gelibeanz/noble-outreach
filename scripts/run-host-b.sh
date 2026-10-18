#!/bin/bash
# Like run-host.sh but isolated: own prefix (work/prefix-b) and exe copy (work/run-b), for a second agent/run.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-b"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
mkdir -p "$ROOT/work/run-b"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-b/reach-host.exe"
cd "$ROOT"
exec umu-run "$ROOT/work/run-b/reach-host.exe" --mcc "$MCC_WIN" "$@"
