#!/usr/bin/env bash
# Render the launcher icon (src/reach-launcher/assets/icon.svg) into the files the build uses:
#   icon.ico       Windows exe icon (16-256 px), embedded by src/reach-launcher/build.rs
#   icon.png       256 px, for Linux desktop entries
#   icon-64.rgba   64x64 raw RGBA, the launcher window's icon (no image decoder needed at runtime)
# Needs inkscape and ImageMagick. Run after editing the SVG and commit the results.
set -euo pipefail
dir="$(cd "$(dirname "$0")/.." && pwd)/src/reach-launcher/assets"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for n in 16 24 32 48 64 128 256; do
  inkscape "$dir/icon.svg" --export-type=png --export-filename="$tmp/$n.png" -w "$n" -h "$n" >/dev/null 2>&1
done
magick "$tmp"/{16,24,32,48,64,128,256}.png "$dir/icon.ico"
cp "$tmp/256.png" "$dir/icon.png"
magick "$tmp/64.png" -depth 8 RGBA:"$dir/icon-64.rgba"
ls -la "$dir"
