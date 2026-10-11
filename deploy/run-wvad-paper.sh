#!/usr/bin/env bash
# run-wvad-paper.sh v1.0.0
# Williams Variable A/D Pressure (WVAD) CRUDEOILM PAPER run: live Kite market data, orders
# filled by the native Kite mock (never touches Zerodha's order API). Same JSON config as live.
set -euo pipefail
cd /home/ubuntu/RustNautilasProject
BIN="./target/release/kite-node"
PORTFOLIO="config/portfolio-production.json"
SLOT="crudeoilm-wvad-202610"
[[ -x "$BIN" ]] || { echo 'Release binary missing: cargo build --release --features live-orders' >&2; exit 1; }
mkdir -p logs
LOG="logs/wvad-paper-$(TZ=Asia/Kolkata date +%F).jsonl"
echo "Live state -> dashboard Redis (config/dashboard.json); JSON events -> $LOG"
exec "$BIN" native-wvad-paper "$PORTFOLIO" "$SLOT" >> "$LOG"
