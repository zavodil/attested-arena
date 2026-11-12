#!/bin/bash
# Builds the judge for wasm32-wasip2. The wasm's sha256 is the judge's
# identity: OutLayer derives the judge's keys from it, and the arena pins the
# signing key derived for it. So the build is reproducible: source paths are
# rewritten to fixed prefixes, and the toolchain (../rust-toolchain.toml) and
# Cargo.lock are pinned — the same commit gives the same bytes.
set -e
cd "$(dirname "$0")/.."

WASM="target/wasm32-wasip2/release/arena-judge.wasm"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"

RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$CARGO_HOME_DIR=/cargo --remap-path-prefix=$PWD=/build" \
    cargo build -p arena-judge --target wasm32-wasip2 --release

grep -qa 'outlayer.manifest' "$WASM" || { echo "ERROR: the manifest section is missing"; exit 1; }
if strings "$WASM" | grep -q -e "$HOME" -e "$PWD"; then
    echo "ERROR: a local path leaked into the build"; exit 1
fi
mkdir -p dist
cp "$WASM" dist/arena-judge.wasm
echo "judge: dist/arena-judge.wasm, $(wc -c < dist/arena-judge.wasm) bytes"
echo "sha256: $(shasum -a 256 dist/arena-judge.wasm | cut -d' ' -f1)"
