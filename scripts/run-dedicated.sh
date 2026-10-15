#!/bin/bash
# Milestone 3a check: a dedicated reach-host server (no local player, hidden window) plus N clients that join
# the running game late, staggered. Own prefix (work/prefix-ded), one folder per instance
# (work/run-ded/{server,c1,c2,...}: exe copy, log, reach-data, frame captures).
#   CLIENTS=2 FIRST=20 STAGGER=15 MAX=4 MAP=forge_world scripts/run-dedicated.sh [server args...]
#   CLIENT_ARGS="--hidden --size 320x180"           extra args for every client
#   CLIENT_ARGS_<n>="--test-input scripts/x.txt"  extra args for client n
#   SERVER_ONLY=1   only the server
#   NO_RESTART=1    do not restart the server when it exits (it exits once the last player has left; umu does not
#                   pass the exe's exit code through, so any exit restarts it)
# Server UDP 127.0.0.1:47900, client n 127.0.0.1:47900+n (away from run-pair's 47800 range).
# The server runs in its own prefix (work/prefix-ded-srv): umu-run only returns once every process in its prefix
# has exited, so sharing the clients' prefix would stop the restart loop from noticing the server exit.
# Stop: WINEPREFIX=work/prefix-ded and WINEPREFIX=work/prefix-ded-srv, <proton>/files/bin/wineserver -k
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-ded"
export GAMEID="umu-976730"
export UMU_RUNTIME_UPDATE=0  # a runtime update mid-run blocks later clients
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
MAP="${MAP:-forge_world}"
CLIENTS="${CLIENTS:-2}"
FIRST="${FIRST:-20}"
STAGGER="${STAGGER:-15}"
MAX="${MAX:-4}"
PORT=47900
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
dirs=(server)
[ -z "$SERVER_ONLY" ] && for n in $(seq 1 "$CLIENTS"); do dirs+=("c$n"); done
for d in "${dirs[@]}"; do
  mkdir -p "$ROOT/work/run-ded/$d"
  rm -f "$ROOT/work/run-ded/$d"/frame-*.bmp "$ROOT/work/run-ded/$d"/*.bmp
  cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-ded/$d/reach-host.exe"
done
cd "$ROOT"
(
  while true; do
    WINEPREFIX="$ROOT/work/prefix-ded-srv" umu-run "$ROOT/work/run-ded/server/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" --dedicated --max-players "$MAX" \
      --bind "127.0.0.1:$PORT" "$@" >> "$ROOT/work/run-ded/server/stdout.txt" 2>&1
    code=$?
    [ -n "$NO_RESTART" ] && break
    cp "$ROOT/work/run-ded/server/reach-host.log" "$ROOT/work/run-ded/server/reach-host.prev.log"
    echo "server exited (code $code), restarting" >> "$ROOT/work/run-ded/server/stdout.txt"
    sleep 2
  done
) &
echo "server loop pid $!"
[ -n "$SERVER_ONLY" ] && { wait; exit; }
sleep "$FIRST"
for n in $(seq 1 "$CLIENTS"); do
  extra="CLIENT_ARGS_$n"
  # shellcheck disable=SC2086
  PROTON_VERB=runinprefix umu-run "$ROOT/work/run-ded/c$n/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" \
    --net-role client --instance "$n" --players "$MAX" --to-dedicated --bind "127.0.0.1:$((PORT + n))" \
    --peer "0=127.0.0.1:$PORT" $CLIENT_ARGS ${!extra} > "$ROOT/work/run-ded/c$n/stdout.txt" 2>&1 &
  echo "client $n pid $! (t=$(( FIRST + (n - 1) * STAGGER )) s)"
  [ "$n" -lt "$CLIENTS" ] && sleep "$STAGGER"
done
wait
