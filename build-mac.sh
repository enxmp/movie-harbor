#!/bin/sh
set -eu
cd "$(dirname "$0")"
npm ci
npm run build
cd src-tauri
cargo build --release --features custom-protocol -j 2
printf '\nBuilt: %s/target/release/movie-harbor\n' "$PWD"
