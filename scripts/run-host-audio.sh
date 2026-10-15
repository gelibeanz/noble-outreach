#!/bin/bash
# Like run-host.sh but isolated for audio work: own prefix (work/prefix-audio) and exe copy (work/run-audio).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-audio"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
mkdir -p "$ROOT/work/run-audio"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
if [ -z "$NO_COPY" ]; then cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-audio/reach-host.exe"; fi
cd "$ROOT"
exec umu-run "$ROOT/work/run-audio/reach-host.exe" --mcc "$MCC_WIN" "$@"
