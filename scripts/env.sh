# Build environment for the gate scripts. Source it from the repository root.
#
# A pinned toolchain unpacked at .tools/toolchain and a crate cache at .cargo
# take precedence when they exist (both live in the main checkout, so git
# worktrees share them). Without them the rustup toolchain on PATH is used.
CHEWVY_ROOT="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "$PWD/.git")")"
if [ -x "$CHEWVY_ROOT/.tools/toolchain/bin/cargo" ]; then
  export PATH="$CHEWVY_ROOT/.tools/toolchain/bin:/usr/bin:/bin"
  export RUSTC="$CHEWVY_ROOT/.tools/toolchain/bin/rustc"
  export RUSTDOC="$CHEWVY_ROOT/.tools/toolchain/bin/rustdoc"
fi
if [ -d "$CHEWVY_ROOT/.cargo/registry" ]; then
  export CARGO_HOME="$CHEWVY_ROOT/.cargo"
fi
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
# The gates never touch the network: run `cargo fetch --locked` once first.
export CARGO_NET_OFFLINE=true
