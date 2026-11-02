#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT/rust"
cargo build --release

mkdir -p "$ROOT/bin"
cp target/release/jb "$ROOT/bin/jb"
chmod +x "$ROOT/bin/jb"

file "$ROOT/bin/jb"
"$ROOT/bin/jb" --version
