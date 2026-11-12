#!/bin/bash
# Builds the arena contract reproducibly: inside a pinned image
# (cargo-near 0.13.4, Rust 1.85.0, by digest), with the repository mounted at
# /build, so every path the compiler sees — and every hash cargo derives from
# a path — is the same on any machine. The same commit gives the same bytes;
# compare the printed NEAR code hash with the one NEAR reports for a season
# account.
set -e
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
IMAGE="sourcescan/cargo-near@sha256:a9d8bee7b134856cc8baa142494a177f2ba9ecfededfcdd38f634e14cca8aae2"
docker run --rm --platform linux/amd64 -v "$ROOT":/build -w /build/contract \
    -e CARGO_TARGET_DIR=/build/contract/target/docker "$IMAGE" \
    cargo near build non-reproducible-wasm --no-abi >/dev/null 2>&1
WASM=target/docker/near/attested_arena.wasm
mkdir -p ../dist && cp "$WASM" ../dist/attested_arena.wasm
python3 - "$WASM" <<'PY'
import hashlib, sys
d = hashlib.sha256(open(sys.argv[1], "rb").read()).digest()
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n, o = int.from_bytes(d, "big"), ""
while n:
    n, r = divmod(n, 58); o = B58[r] + o
print(f"contract: dist/attested_arena.wasm\nsha256: {d.hex()}\nNEAR code hash: {o}")
PY
