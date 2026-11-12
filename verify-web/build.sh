#!/bin/bash
# Builds the report's in-browser checker (the judge's own checks, as
# WebAssembly) into web/verify/arena.wasm.
set -e
cd "$(dirname "$0")/.."
cargo build -p arena-verify-web --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/arena_verify_web.wasm web/verify/arena.wasm
echo "web/verify/arena.wasm: $(wc -c < web/verify/arena.wasm) bytes"
