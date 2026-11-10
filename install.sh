#!/usr/bin/env bash
# Build dtem from source and install it for the current user.
# (A prebuilt binary doesn't need this: just run `./dtem --install`.)
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
exec target/release/dtem --install "$@"
