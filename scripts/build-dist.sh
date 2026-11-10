#!/usr/bin/env bash
# Build portable release artifacts into dist/:
#   dtem-<ver>-x86_64 / dtem-<ver>-aarch64   single binaries (glibc >= 2.30: Ubuntu 20.04+, Debian 11+, …)
#   dtem_<ver>-1_amd64.deb / _arm64.deb      Debian/Ubuntu packages
# Needs: cargo-zigbuild + zig (`pip install ziglang`), cargo-deb, rustup targets
#        x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu.
set -euo pipefail
cd "$(dirname "$0")/.."
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p dist
for t in x86_64 aarch64; do
  cargo zigbuild --release --target "$t-unknown-linux-gnu.2.31"
  cp "target/$t-unknown-linux-gnu/release/dtem" "dist/dtem-$ver-$t"
  cargo deb --target "$t-unknown-linux-gnu" --no-build -o dist/
done
( cd dist && sha256sum dtem-"$ver"-* dtem_"$ver"* > SHA256SUMS )
ls -la dist
