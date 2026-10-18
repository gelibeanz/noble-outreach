#!/bin/bash
# NAT punch check on one PC: local reach-master (HTTP 49110, rendezvous UDP 49111, --allow-private), a listen host
# (--listen, UDP 49100, registers with the master and the rendezvous), then a joiner with --join-server that skips
# the direct try (REACH_NAT_FORCE_PUNCH=1) so the rendezvous introduction and both punch bursts run. Prints the
# nat/upnp/join lines of both logs. Own prefix work/prefix-nat, folders work/run-nat/{host,client}; stops the prefix
# and the master at the end unless KEEP=1. Real NAT is not exercised here (everything is 127.0.0.1).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-nat"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
export REACH_UI_HIDDEN=1
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
MASTER="http://127.0.0.1:49110"
WAIT="${WAIT:-50}"
TARGET="$ROOT/work/target-nat"
if [ -z "$NO_BUILD" ]; then
  (cd "$ROOT" && CARGO_TARGET_DIR="$TARGET" cargo build --release -p reach-master \
    && CARGO_TARGET_DIR="$TARGET" cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1
fi
for r in host client; do
  mkdir -p "$ROOT/work/run-nat/$r"
  rm -f "$ROOT/work/run-nat/$r"/frame-*.bmp "$ROOT/work/run-nat/$r"/at-*.bmp "$ROOT/work/run-nat/$r/reach-host.log"
  cp "$TARGET/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-nat/$r/reach-host.exe"
done
cd "$ROOT"
"$TARGET/release/reach-master" --listen 127.0.0.1:49110 --allow-private --rendezvous-listen 127.0.0.1:49111 \
  --rendezvous-public 127.0.0.1:49111 > work/run-nat/master.log 2>&1 &
MASTER_PID=$!
cleanup() {
  kill "$MASTER_PID" 2>/dev/null
  [ -n "$KEEP" ] || "$PROTONPATH/files/bin/wineserver" -k 2>/dev/null
}
trap cleanup EXIT
sleep 1
echo "== /api/rendezvous: $(curl -s $MASTER/api/rendezvous)"
umu-run "$ROOT/work/run-nat/host/reach-host.exe" --mcc "$MCC_WIN" --hidden --listen --players 4 --bind 0.0.0.0:49100 \
  --map forge_world --game-variant "$MCC_WIN\\haloreach\\game_variants\\slayer_054.bin" \
  --master "$MASTER" --server-name "NAT test" ${HOST_ARGS:-} > work/run-nat/host/stdout.txt 2>&1 &
sleep "${HOST_DELAY:-20}"
echo "== master list: $(curl -s $MASTER/api/servers)"
REACH_NAT_FORCE_PUNCH=1 REACH_CAPTURE_AT="${CAPTURE_AT:-2,4,6}" PROTON_VERB=runinprefix REACH_MASTER="$MASTER" \
  umu-run "$ROOT/work/run-nat/client/reach-host.exe" --mcc "$MCC_WIN" --hidden --join-server "$MASTER" 0 \
  > work/run-nat/client/stdout.txt 2>&1 &
sleep "$WAIT"
for r in host client; do
  echo "== $r log:"
  grep -E "nat:|upnp|join-server|master:|map loaded|roster|net: new peer|FATAL" "$ROOT/work/run-nat/$r/reach-host.log" | head -40
done
echo "== master log:"; cat work/run-nat/master.log
