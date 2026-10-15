#!/usr/bin/env bash
# Install the reach-tracer proxy DLL into the MCC haloreach folder.
# The real engine is moved to haloreach_real.dll; the tracer forwards to it.
set -euo pipefail

EXPECTED_SHA256=738dd2d24ea3aea12e1ee9aa4a61094bf116027d42004c35a19e5048608b0894

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TRACER="$PROJECT_DIR/target/x86_64-pc-windows-gnu/release/haloreach.dll"
MCC_DIR="${MCC_DIR:-$HOME/.local/share/Steam/steamapps/common/Halo The Master Chief Collection}"
TARGET="$MCC_DIR/haloreach"

sha256_of() {
    sha256sum "$1" | awk '{print $1}'
}

if [ ! -f "$TRACER" ]; then
    echo "error: tracer DLL not found: $TRACER" >&2
    echo "build it first: cd \"$PROJECT_DIR\" && cargo zigbuild --release --target x86_64-pc-windows-gnu" >&2
    exit 1
fi

if [ ! -d "$TARGET" ]; then
    echo "error: MCC haloreach folder not found: $TARGET" >&2
    echo "set MCC_DIR to your MCC install folder and retry" >&2
    exit 1
fi

if [ -f "$TARGET/haloreach_real.dll" ]; then
    # Already installed: only refresh the proxy copy.
    cp -f "$TRACER" "$TARGET/haloreach.dll"
    echo "tracer already installed; refreshed proxy haloreach.dll from the current build"
else
    if [ ! -f "$TARGET/haloreach.dll" ]; then
        echo "error: $TARGET/haloreach.dll is missing; is MCC installed?" >&2
        exit 1
    fi
    actual="$(sha256_of "$TARGET/haloreach.dll")"
    if [ "$actual" != "$EXPECTED_SHA256" ]; then
        echo "error: haloreach.dll hash does not match the frozen MCC build; refusing to install" >&2
        echo "  expected: $EXPECTED_SHA256" >&2
        echo "  actual:   $actual" >&2
        exit 1
    fi
    mv "$TARGET/haloreach.dll" "$TARGET/haloreach_real.dll"
    cp -f "$TRACER" "$TARGET/haloreach.dll"
    echo "installed tracer: real engine moved to haloreach_real.dll, proxy copied to haloreach.dll"
fi

echo
echo "trace output will appear in: $TARGET/reach-trace/"
echo
echo "next steps:"
echo "  1. launch MCC with Easy Anti-Cheat DISABLED"
echo "  2. start a Reach custom game"
echo "  3. play for about a minute, then quit"
