#!/bin/bash
# Milestone 3a check: a dedicated reach-host server (no local player, hidden window) plus N clients that join
# the running game late, staggered. Own prefix (work/prefix-ded), one folder per instance
# (work/run-$RUN/{server,c1,c2,...}: exe copy, log, reach-data, frame captures).
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
RUN="${RUN:-ded}"  # RUN=x → work/run-x, work/prefix-x(-srv): parallel agents each use their own
export WINEPREFIX="$ROOT/work/prefix-$RUN"
# Test games are silent (no Wine audio driver) so they never play over the desktop; SOUND=1 keeps sound.
[ -n "${SOUND:-}" ] || export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winepulse.drv=d;winealsa.drv=d"
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
PORT="${PORT:-47900}"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
dirs=(server)
[ -z "$SERVER_ONLY" ] && for n in $(seq 1 "$CLIENTS"); do dirs+=("c$n"); done
for d in "${dirs[@]}"; do
  mkdir -p "$ROOT/work/run-$RUN/$d"
  rm -f "$ROOT/work/run-$RUN/$d"/frame-*.bmp "$ROOT/work/run-$RUN/$d"/*.bmp
  cp "${CARGO_TARGET_DIR:-$ROOT/target}/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-$RUN/$d/reach-host.exe"
done
cd "$ROOT"
(
  while true; do
    WINEPREFIX="$ROOT/work/prefix-$RUN-srv" umu-run "$ROOT/work/run-$RUN/server/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" --dedicated --max-players "$MAX" \
      --bind "127.0.0.1:$PORT" "$@" >> "$ROOT/work/run-$RUN/server/stdout.txt" 2>&1
    code=$?
    [ -n "$NO_RESTART" ] && break
    cp "$ROOT/work/run-$RUN/server/reach-host.log" "$ROOT/work/run-$RUN/server/reach-host.prev.log"
    echo "server exited (code $code), restarting" >> "$ROOT/work/run-$RUN/server/stdout.txt"
    sleep 2
  done
) &
echo "server loop pid $!"
[ -n "$SERVER_ONLY" ] && { wait; exit; }
sleep "$FIRST"
# runinprefix cannot create a prefix: seed a new client prefix from the main checkout's work/prefix
if [ ! -d "$WINEPREFIX/pfx/drive_c" ]; then
  main="$(dirname "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)")"
  cp -a --reflink=auto "$main/work/prefix/." "$WINEPREFIX/"
fi
for n in $(seq 1 "$CLIENTS"); do
  extra="CLIENT_ARGS_$n"
  # shellcheck disable=SC2086
  PROTON_VERB=runinprefix umu-run "$ROOT/work/run-$RUN/c$n/reach-host.exe" --mcc "$MCC_WIN" --map "$MAP" \
    --net-role client --instance "$n" --players "$MAX" --to-dedicated --bind "127.0.0.1:$((PORT + n))" \
    --peer "0=127.0.0.1:$PORT" $CLIENT_ARGS ${!extra} > "$ROOT/work/run-$RUN/c$n/stdout.txt" 2>&1 &
  echo "client $n pid $! (t=$(( FIRST + (n - 1) * STAGGER )) s)"
  [ "$n" -lt "$CLIENTS" ] && sleep "$STAGGER"
done
wait
