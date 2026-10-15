#!/bin/bash
# Frontend runs, isolated: own prefix (work/prefix-ui) and exe copy (work/run-ui). No args = main menu.
#   scripts/run-ui.sh                                   menu (PLAY / SERVER BROWSER / HOST GAME / SETTINGS)
#   REACH_UI_SHOT=home,play-screen,browser,host,settings NO_BUILD=1 scripts/run-ui.sh
#       captures each menu screen to work/run-ui/ui-<screen>.bmp, then quits
#   REACH_UI_SHOT=play,pause,resume,leave scripts/run-ui.sh --test-input scripts/test-ui-pause.txt
#       menu -> local game -> Esc -> ui-pause.bmp -> RESUME -> ui-after-resume.bmp -> Esc -> LEAVE GAME
#       -> new process -> ui-relaunched.bmp (the game's log is kept as reach-host-game.log)
# Stop: WINEPREFIX=work/prefix-ui <proton>/files/bin/wineserver -k
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export WINEPREFIX="$ROOT/work/prefix-ui"
export GAMEID="umu-976730"
export UMU_RUNTIME_UPDATE=0
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64"
MCC_LINUX="${MCC_LINUX:-$ROOT/work/mcc}"
MCC_WIN="Z:${MCC_LINUX//\//\\}"
mkdir -p "$ROOT/work/run-ui"
if [ -z "$NO_BUILD" ]; then (cd "$ROOT" && cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host) || exit 1; fi
if [ -z "$NO_COPY" ]; then cp "$ROOT/target/x86_64-pc-windows-gnu/release/reach-host.exe" "$ROOT/work/run-ui/reach-host.exe"; fi
cd "$ROOT"
exec umu-run "$ROOT/work/run-ui/reach-host.exe" --mcc "$MCC_WIN" "$@"
