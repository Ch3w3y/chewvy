#!/bin/bash
# The four gates every change must pass: fmt, check (clippy), test, build.
# Usage, from the repository root: scripts/check.sh <fmt|check|test|build>
set -eu
source scripts/env.sh
case "${1:-}" in
 fmt) cargo fmt --all -- --check ;;
 check) cargo clippy --offline --locked --all-targets -- -D warnings ;;
 test) cargo test --offline --locked --all-targets ;;
 build) cargo build --offline --locked ;;
 *) echo "usage: scripts/check.sh <fmt|check|test|build>" >&2; exit 2 ;;
esac
