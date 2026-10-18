#!/bin/bash
# Milestone 3b check: master server + server browser. Starts reach-master natively on 127.0.0.1:47880
# (--allow-private), lists it with --browse (empty), starts a host with --master, lists again, then starts
# a client with --join-server and prints the evidence lines. Own prefix work/prefix-browse, instance
# folders work/run-browse/{host,client}. Kills only the master it started; stop the prefix with
#   WINEPREFIX=work/prefix-browse <proton>/files/bin/wineserver -k   (done at the end unless KEEP=1)
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-browse"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
export GAMEID="umu-976730"
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
MAP="${MAP:-forge_world}"
MASTER="http://127.0.0.1:47880"
WAIT="${WAIT:-45}"
EXE="$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe"
if [ -z "$NO_BUILD" ]; then
  (cd "$ROOT" && cargo build --release -p reach-master && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1
fi
for r in host client; do
  mkdir -p "$ROOT/work/run-browse/$r"
  rm -f "$ROOT/work/run-browse/$r"/frame-*.bmp "$ROOT/work/run-browse/$r/reach-host.log"
  cp "$EXE" "$ROOT/work/run-browse/$r/reach-host.exe"
done
cd "$ROOT"
"$ROOT/target/release/reach-master" --listen 127.0.0.1:47880 --allow-private > work/run-browse/master.log 2>&1 &
MASTER_PID=$!
cleanup() {
  kill "$MASTER_PID" 2>/dev/null
  [ -n "$KEEP" ] || "$PROTONPATH/files/bin/wineserver" -k 2>/dev/null
}
trap cleanup EXIT
sleep 1
echo "== master pid $MASTER_PID; healthz: $(curl -s $MASTER/healthz)"
echo "== --browse (empty list, under Wine):"
browse() { rm -f "$ROOT/work/run-browse/host/browse.txt"; "$@" --browse "$MASTER" >/dev/null 2>&1; cat "$ROOT/work/run-browse/host/browse.txt"; }
browse umu-run "$ROOT/work/run-browse/host/reach-host.exe"

umu-run "$ROOT/work/run-browse/host/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" --net-role host --instance 0 --players 2 \
  --master "$MASTER" --server-name "Browse test" > work/run-browse/host/stdout.txt 2>&1 &
sleep 8
echo "== --browse with the host registered:"
PROTON_VERB=runinprefix browse umu-run "$ROOT/work/run-browse/host/reach-host.exe"
echo "== curl GET /api/servers:"; curl -s $MASTER/api/servers; echo

PROTON_VERB=runinprefix umu-run "$ROOT/work/run-browse/client/reach-host.exe" --mcc "$MCC_WIN" --join-server "$MASTER" 0 \
  > work/run-browse/client/stdout.txt 2>&1 &
sleep "$WAIT"
for r in host client; do
  echo "== $r log:"
  grep -E "master:|join-server|map loaded|net heartbeat|roster|FATAL" "$ROOT/work/run-browse/$r/reach-host.log" | tail -12
done
echo "== master list now:"; curl -s $MASTER/api/servers; echo
