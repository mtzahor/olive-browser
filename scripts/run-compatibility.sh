#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
OLIVE_LIVE_COMPAT=1 cargo test --locked --all-features --test compatibility
