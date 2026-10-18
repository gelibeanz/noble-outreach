#!/bin/sh
# Runs reach-host.exe --dedicated in a loop under Wine + DXVK on a hidden Xvfb display.
# The server exits when the last player leaves (the engine cannot start a game with nobody in it),
# so every exit just starts a fresh one.
#
#   REACH_MODE         slayer (default) | firefight | campaign
#   REACH_MAP          map (default by mode: forge_world, ff10_prototype, m10)
#   REACH_VARIANT      game variant file name in haloreach/game_variants (slayer/firefight; default by reach-host)
#   REACH_DIFFICULTY   easy | normal | heroic | legendary (firefight/campaign; default normal)
#   REACH_MAX_PLAYERS  default 16 (Firefight and campaign: at most REACH_COOP_MAX)
#   REACH_COOP_MAX     co-op player cap for Firefight/campaign, 4 (default, the engine's own) up to 8
#   REACH_NAME         name in the server list (default "Noble Outreach <Mode>")
#   REACH_MASTER       master URL(s) to list the server on, comma-separated. Empty (default) or "off" = not
#                      listed: players add it by address (Direct Connect / Favourites) or find it on the LAN.
#   REACH_PUBLIC_ADDR  public ip:port players use (a UDP tunnel address, or your forwarded port). Empty = the
#                      master fills in the address it sees the heartbeat from, with port 47800.
#   REACH_ARGS         extra reach-host arguments, split on spaces
#   REACH_RENDER       off = --no-render (set by the image's GPU=cpu build: lavapipe, nothing drawn), on = draw
#   REACH_PLAYLIST     playlist file in /playlists: players vote on the next game (overrides mode/map)
set -eu

: "${REACH_MODE:=slayer}"
: "${REACH_VARIANT:=}"
: "${REACH_DIFFICULTY:=normal}"
: "${REACH_MAX_PLAYERS:=16}"
: "${REACH_COOP_MAX:=4}"
COOP_ARGS=""
[ "$REACH_COOP_MAX" -gt 4 ] && COOP_ARGS="--coop-max $REACH_COOP_MAX"
# Mode -> reach-host flags. Firefight and campaign are co-op modes: 4 players, or up to 8 with REACH_COOP_MAX.
case "$REACH_MODE" in
  slayer|multiplayer|mp)
    : "${REACH_MAP:=forge_world}"; REACH_MODE_TITLE=Slayer
    set -- --map "$REACH_MAP" ;;
  firefight|ff)
    : "${REACH_MAP:=ff10_prototype}"; REACH_MODE_TITLE=Firefight
    [ "$REACH_MAX_PLAYERS" -gt "$REACH_COOP_MAX" ] && REACH_MAX_PLAYERS=$REACH_COOP_MAX
    set -- --firefight "$REACH_MAP" --difficulty "$REACH_DIFFICULTY" ;;
  campaign|coop)
    : "${REACH_MAP:=m10}"; REACH_MODE_TITLE=Campaign
    [ "$REACH_MAX_PLAYERS" -gt "$REACH_COOP_MAX" ] && REACH_MAX_PLAYERS=$REACH_COOP_MAX
    set -- --campaign "$REACH_MAP" --difficulty "$REACH_DIFFICULTY" ;;
  *) echo "[reach-server] REACH_MODE=$REACH_MODE: expected slayer, firefight or campaign" >&2; exit 2 ;;
esac
if [ -n "$REACH_VARIANT" ] && [ "$REACH_MODE_TITLE" != Campaign ]; then
  set -- "$@" --game-variant "Z:\\mcc\\haloreach\\game_variants\\$REACH_VARIANT"
fi
MODE_ARGS="$*"
: "${REACH_PLAYLIST:=}"
if [ -n "$REACH_PLAYLIST" ]; then
  [ -f "/playlists/$REACH_PLAYLIST" ] || { echo "[reach-server] REACH_PLAYLIST=$REACH_PLAYLIST: not in /playlists" >&2; exit 2; }
  : "${REACH_NAME:=Noble Outreach $REACH_MODE_TITLE Playlist}"
  REACH_ARGS="${REACH_ARGS:-} --playlist Z:\\playlists\\$REACH_PLAYLIST"
fi
: "${REACH_NAME:=Noble Outreach $REACH_MODE_TITLE}"
: "${REACH_PUBLIC_ADDR:=}"
: "${REACH_MASTER:=}"
: "${REACH_ARGS:=}"
: "${REACH_RENDER:=on}"
[ "$REACH_RENDER" = off ] && REACH_ARGS="$REACH_ARGS --no-render"

if [ ! -f "$WINEPREFIX/system.reg" ]; then
  echo "[reach-server] creating Wine prefix in $WINEPREFIX"
  mkdir -p "$WINEPREFIX"
  wineboot --init
  wineserver -w
fi
# Xvfb's RandR modes have no refresh rate, and DXVK's present divides by it (crash in dxgi.dll).
# Without XRandR/XVidMode Wine reports a fixed 60 Hz mode.
if ! grep -q '"UseXRandR"="N"' "$WINEPREFIX/user.reg"; then
  wine reg add 'HKCU\Software\Wine\X11 Driver' /v UseXRandR /t REG_SZ /d N /f >/dev/null 2>&1
  wine reg add 'HKCU\Software\Wine\X11 Driver' /v UseXVidMode /t REG_SZ /d N /f >/dev/null 2>&1
  wineserver -w
fi
# DXVK into the prefix every start (cheap, and picks up image updates).
cp /opt/dxvk/x64/*.dll "$WINEPREFIX/drive_c/windows/system32/"

# reach-host writes its log and captures next to the exe, so run it from the writable volume.
mkdir -p /data/run
cp /opt/reach/reach-host.exe /data/run/reach-host.exe

# A container restart keeps /tmp, and a stale lock stops Xvfb from starting.
rm -f /tmp/.X99-lock /tmp/.X11-unix/X99
Xvfb :99 -screen 0 640x480x24 -nolisten tcp >/dev/null 2>&1 &
export DISPLAY=:99
sleep 1

touch /data/run/reach-host.log
tail -n 0 -F /data/run/reach-host.log 2>/dev/null &

set -f
while true; do
  # shellcheck disable=SC2086
  set -- --dedicated --mcc 'Z:\mcc' $MODE_ARGS --max-players "$REACH_MAX_PLAYERS" $COOP_ARGS \
    --bind 0.0.0.0:47800 --server-name "$REACH_NAME" $REACH_ARGS
  if [ -n "$REACH_MASTER" ] && [ "$REACH_MASTER" != off ]; then
    set -- "$@" --master "$REACH_MASTER"
    [ -n "$REACH_PUBLIC_ADDR" ] && set -- "$@" --public-addr "$REACH_PUBLIC_ADDR"
  fi
  echo "[reach-server] $(date -u +%H:%M:%S) starting reach-host $*"
  [ -f /data/run/reach-host.log ] && cp /data/run/reach-host.log /data/run/reach-host.prev.log
  code=0
  (cd /data/run && wine reach-host.exe "$@") >> /data/run/stdout.txt 2>&1 || code=$?
  echo "[reach-server] $(date -u +%H:%M:%S) reach-host exited (code $code), restarting"
  wineserver -k 2>/dev/null || true
  sleep 2
done
