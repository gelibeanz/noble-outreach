#!/usr/bin/env bash
# Remove the reach-tracer proxy and restore the real MCC haloreach.dll.
set -euo pipefail

EXPECTED_SHA256=738dd2d24ea3aea12e1ee9aa4a61094bf116027d42004c35a19e5048608b0894

MCC_DIR="${MCC_DIR:-$HOME/.local/share/Steam/steamapps/common/Halo The Master Chief Collection}"
TARGET="$MCC_DIR/haloreach"

sha256_of() {
    sha256sum "$1" | awk '{print $1}'
}

if [ ! -f "$TARGET/haloreach_real.dll" ]; then
    echo "nothing to restore: $TARGET/haloreach_real.dll not found (tracer not installed?)"
    exit 0
fi

rm -f "$TARGET/haloreach.dll"
mv "$TARGET/haloreach_real.dll" "$TARGET/haloreach.dll"

actual="$(sha256_of "$TARGET/haloreach.dll")"
if [ "$actual" != "$EXPECTED_SHA256" ]; then
    echo "error: restored haloreach.dll hash does not match the frozen MCC build" >&2
    echo "  expected: $EXPECTED_SHA256" >&2
    echo "  actual:   $actual" >&2
    exit 1
fi

echo "restored real haloreach.dll, sha256 $actual (matches frozen build)"
echo "left $TARGET/reach-trace/ in place (trace output is kept)"
