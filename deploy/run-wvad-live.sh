#!/usr/bin/env bash
# run-wvad-live.sh v1.0.0
# Williams Variable A/D Pressure (WVAD) CRUDEOILM LIVE run: places REAL Zerodha orders
# (lots, TP1/TP2 lots, timeframe, blackouts all from config/wvad-crudeoilm.json; MIS,
# square-off from the same file).
# Gates: --features live-orders build, slot enabled + live_orders_enabled in
# config/portfolio-production.json, live_orders_enabled and max_lots >= lots in
# config/kite-production.json, and the operator typing LIVE below.
# Only one slot may trade live on the account at a time (see doc/PORTFOLIO_SLOTS.md).
set -euo pipefail
cd /home/ubuntu/RustNautilasProject
BIN="./target/release/kite-node"
PORTFOLIO="config/portfolio-production.json"
SLOT="crudeoilm-wvad-202610"
BROKER="config/kite-production.json"
[[ -x "$BIN" ]] || { echo 'Release binary missing: cargo build --release --features live-orders' >&2; exit 1; }
[[ -f "$PORTFOLIO" && -f "$BROKER" ]] || { echo 'Live configuration files missing.' >&2; exit 1; }
"$BIN" native-wvad-live-preflight "$PORTFOLIO" "$SLOT" "$BROKER"
LOTS=$(python3 -c 'import json;print(json.load(open("config/wvad-crudeoilm.json"))["lots"])')
echo "WARNING: This will place REAL Zerodha Kite orders for CRUDEOILM (${LOTS} lots, MIS)."
read -r -p 'Type LIVE to proceed: ' CONFIRM
[[ "$CONFIRM" == 'LIVE' ]] || { echo 'Cancelled.'; exit 1; }
mkdir -p logs
LOG="logs/wvad-live-$(TZ=Asia/Kolkata date +%F).jsonl"
echo "Live state -> dashboard Redis (config/dashboard.json); JSON events -> $LOG"
# stdout (JSON) goes straight to the file: no pipe, so Ctrl+C reaches only the
# runner, which flattens and stops cleanly.
exec "$BIN" native-wvad-live "$PORTFOLIO" "$SLOT" "$BROKER" >> "$LOG"
