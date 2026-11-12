#!/bin/bash
# Type-check the workspace for another desktop target without linking.
# windows: needs clang, llvm-ar and mingw-w64-headers on the host.
# macOS cannot be checked here (no SDK headers for ring); CI covers it.
set -eu
source scripts/env.sh
export PATH="$PATH:/usr/bin"
export PLEX_DISABLE_MPV_GUARD=1
case "${1:-}" in
 windows)
  export CC_x86_64_pc_windows_gnu=clang
  export CFLAGS_x86_64_pc_windows_gnu="--target=x86_64-w64-mingw32 --sysroot=/usr/x86_64-w64-mingw32"
  export AR_x86_64_pc_windows_gnu=llvm-ar
  cargo clippy --offline --locked --target x86_64-pc-windows-gnu --workspace --all-targets -- -D warnings ;;
 *) echo "usage: scripts/xcheck.sh windows" >&2; exit 2 ;;
esac
