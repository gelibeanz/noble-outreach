#!/bin/bash
# Compile our own Megalo game types (modes/*.megalo) into game variant files with ReachVariantTool's headless mode.
#   scripts/build-modes.sh [mode.megalo ...]      → work/modes/<name>.bin
# Each source names its base variant on a "-- base: <file>" line (default slayer_054.bin); the base comes from the
# player's MCC files (MCC_LINUX, default work/mcc) and only its script is replaced. RVT (GPL-3, Windows/Qt) runs under
# Proton's wine in its own silent prefix: work/tools/rvt/<version>/ReachVariantTool.exe in the main checkout
# (download: notes/megalo.md). Nothing here is shipped yet: an experiment, see notes/megalo.md.
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MAIN="$(dirname "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)")"
RVT="${RVT:-$MAIN/work/tools/rvt/v2.3.2/ReachVariantTool.exe}"
MCC_LINUX="${MCC_LINUX:-$MAIN/work/mcc}"
PROTON="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-5-x86_64/files/bin"
export WINEPREFIX="${RVT_PREFIX:-$MAIN/work/prefix-megalo-rvt/pfx}" WINEDEBUG=-all
export WINEDLLOVERRIDES="winepulse.drv=d;winealsa.drv=d"  # silent: a compiler has no business with audio
[ -f "$RVT" ] || { echo "no ReachVariantTool at $RVT (see notes/megalo.md)"; exit 1; }
# A fresh prefix: wineboot once (no window, RVT --headless never creates one)
[ -d "$WINEPREFIX/drive_c" ] || { mkdir -p "$WINEPREFIX"; "$PROTON/wine" wineboot -i >/dev/null 2>&1; }
out="$ROOT/work/modes"
mkdir -p "$out"
srcs=("$@")
[ ${#srcs[@]} -gt 0 ] || srcs=("$ROOT"/modes/*.megalo)
for src in "${srcs[@]}"; do
  name="$(basename "$src" .megalo)"
  base="$(sed -n 's/^-- base: *\([^ ]*\).*/\1/p' "$src" | head -1)"
  base="$MCC_LINUX/haloreach/game_variants/${base:-slayer_054.bin}"
  tmp="$(mktemp -d)"
  cp "$base" "$tmp/base.bin"
  cp "$src" "$tmp/src.megalo"
  # RVT prints [FATAL]/[ERROR]/[WARN:] lines with line and column; it only saves when compilation succeeded
  (cd "$tmp" && "$PROTON/wine" "$RVT" --headless base.bin --recompile src.megalo --dst out.bin 2>&1 | grep -v -E "ntsync|INI file")
  if [ -f "$tmp/out.bin" ]; then cp "$tmp/out.bin" "$out/$name.bin"; echo "$name: $out/$name.bin"; else echo "$name: FAILED"; rm -rf "$tmp"; exit 1; fi
  rm -rf "$tmp"
done
"$PROTON/wineserver" -k 2>/dev/null || true
