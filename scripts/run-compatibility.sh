#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --all-features --example site-report
python3 scripts/compatibility-report.py "$@"
