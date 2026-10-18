#!/bin/bash
# Milestone 2 check: two reach-host instances on one PC in one Slayer match over our UDP transport.
# Own prefix (work/prefix-net) and one folder per instance (work/run-net/{host,client}: exe copy, log,
# reach-data, frame captures). Starts the host, then the client. Extra args go to both instances.
# The client runs with PROTON_VERB=runinprefix: the default verb (waitforexitandrun) would wait for the host to exit.
# Stop: WINEPREFIX=work/prefix-net <proton>/files/bin/wineserver -k
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-net"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
MAP="${MAP:-forge_world}"
DELAY="${DELAY:-3}"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
for r in host client; do
  mkdir -p "$ROOT/work/run-net/$r"
  rm -f "$ROOT/work/run-net/$r"/frame-*.bmp
  cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-net/$r/reach-host.exe"
done
cd "$ROOT"
umu-run "$ROOT/work/run-net/host/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" --local-pair host "$@" \
  > "$ROOT/work/run-net/host/stdout.txt" 2>&1 &
HOST_PID=$!
sleep "$DELAY"
PROTON_VERB=runinprefix umu-run "$ROOT/work/run-net/client/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" --local-pair client "$@" \
  > "$ROOT/work/run-net/client/stdout.txt" 2>&1 &
CLIENT_PID=$!
echo "host pid $HOST_PID, client pid $CLIENT_PID; logs: work/run-net/{host,client}/reach-host.log"
wait
