#!/usr/bin/env bash
# Build the release files for one version (what .github/workflows/release.yml runs on a v* tag).
#
#   scripts/build-release.sh v0.1.0 [OUTDIR]     (default OUTDIR: dist/release-<version>)
#
# Output: NobleOutreach-Launcher.exe, NobleOutreach-Launcher-linux (glibc 2.31+), reach-host.exe (the client the
# launcher installs), NobleOutreach-Server-<version>.tar.gz (reach-host.exe + static reach-master), SHA256SUMS.
# The launcher's build version is the tag, so it matches the GitHub release it updates from. Needs cargo-zigbuild
# + zig and the x86_64-pc-windows-gnu / x86_64-unknown-linux-musl Rust targets. Build paths are remapped so no
# local directory names end up in the binaries.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
ver="${1:?usage: scripts/build-release.sh <version tag, e.g. v0.1.0> [OUTDIR]}"
out="${2:-$root/dist/release-$ver}"
tgt="${CARGO_TARGET_DIR:-$root/target}"
export REACH_LAUNCHER_VERSION="$ver"
# Later prefixes win where they overlap, so the specific ones come after $HOME.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup --remap-path-prefix=$root=/noble-outreach"
cd "$root"
cargo zigbuild --release --locked --target x86_64-pc-windows-gnu -p reach-host -p reach-launcher
cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu.2.31 -p reach-launcher
cargo zigbuild --release --locked --target x86_64-unknown-linux-musl -p reach-master

rm -rf "$out"
mkdir -p "$out/server"
cp "$tgt/x86_64-pc-windows-gnu/release/reach-launcher.exe" "$out/NobleOutreach-Launcher.exe"
cp "$tgt/x86_64-unknown-linux-gnu/release/reach-launcher" "$out/NobleOutreach-Launcher-linux"
cp "$tgt/x86_64-pc-windows-gnu/release/reach-host.exe" "$out/reach-host.exe"
cp "$tgt/x86_64-pc-windows-gnu/release/reach-host.exe" "$tgt/x86_64-unknown-linux-musl/release/reach-master" "$out/server/"
chmod +x "$out/NobleOutreach-Launcher-linux" "$out/server/reach-master"
tar -czf "$out/NobleOutreach-Server-$ver.tar.gz" --owner=0 --group=0 -C "$out/server" reach-host.exe reach-master
rm -rf "$out/server"
# No build machine paths in the binaries.
for f in "$out"/NobleOutreach-Launcher.exe "$out"/NobleOutreach-Launcher-linux "$out"/reach-host.exe "$tgt/x86_64-unknown-linux-musl/release/reach-master"; do
    if grep -a -q -F "$HOME/" "$f"; then echo "build path $HOME left in $f" >&2; exit 1; fi
done
(cd "$out" && sha256sum NobleOutreach-Launcher.exe NobleOutreach-Launcher-linux reach-host.exe "NobleOutreach-Server-$ver.tar.gz" > SHA256SUMS && cat SHA256SUMS)
