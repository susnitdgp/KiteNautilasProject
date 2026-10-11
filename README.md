# Rust NautilusTrader / Zerodha Kite Research Workspace

NautilusTrader 0.64 workspace for Zerodha Kite Connect: the Kite adapter (market data, instruments, history, native execution client) and **WVAD** (Williams Variable A/D Pressure, CRUDEOILM), with backtests, paper runs and gated live runs. Real orders need a `live-orders` build, the slot and broker-settings gates and a typed `LIVE` (see `doc/PORTFOLIO_SLOTS.md`).

Precision Sniper and SATS are **parked**: their model crates stay in `crates/sniper` and `crates/sats` (built and
unit-tested, not used by kite-node); their kite-node runners, configs and scripts are in `backup/` (not built).
`backup/README.md` says how to bring them back. Their last runnable state is the branch `backup-2026-10-11-sniper-sats-wvad`.

## WVAD (Rust port)

`crates/wvad` ports Williams Variable A/D Pressure [MarkitTick] (CC BY-NC-SA 4.0): WVAD sum, adaptive filter,
signal-line crossover with optional ADX / higher-timeframe filters, ATR stop and 1R/2R/3R targets. It runs on the
generic bar-model runner (`slot_config`, `model_strategy`, `model_live`, `model_backtest`). See `doc/WVAD_STRATEGY.md`.

Backtest (read-only, Kite historical candles, no orders):

    ./target/release/kite-node native-wvad-backtest config/wvad-crudeoilm.json 2026-09-19 2026-10-09

Writes `backtest_results/wvad/<symbol>/<run>/summary.txt`, `summary.json` and `trades.csv`.

### Running WVAD on the market (CRUDEOILM slot `crudeoilm-wvad-202610`)

* Paper (live Kite data, Kite mock execution, no broker orders): `deploy/run-wvad-paper.sh`
* Live (REAL Zerodha orders; the slot is enabled for live from 12 Oct 2026):
  1. `cargo build --release --features live-orders`
  2. `config/portfolio-production.json` has the slot `enabled` and `live_orders_enabled` (both on)
  3. local, git-ignored `config/kite-production.json` has `live_orders_enabled: true` and your Kite user ID
  4. `deploy/run-wvad-live.sh` runs the preflight, then asks you to type `LIVE`

Each run covers one trading day: start it after 09:00 IST; it warms the model on broker-finalised history, trades,
flattens on the 23:15 bar and stops. A WebSocket gap, reconnect or invalid packet fails closed (flatten + stop),
any order rejection or position mismatch halts new orders and flattens. Logs (JSON lines) go to `logs/`.
After the contract expiry (2026-10-19), roll the slot (instrument, token, rollover block).

### State outside the process (named from the portfolio manifest)

A run keeps its order records, strategy state and Nautilus cache in memory only; there is no
order journal and no crash recovery (kite-node 2.21.0). Kite is the source of truth: every
live start requires a flat account with no open orders.

| Name | Purpose |
|---|---|
| file `~/.local/state/kite-node/locks/<prefix>-<slot>-<kite user>.lock` | one process per slot and account (OS lock, freed when the process ends; `KITE_LOCK_DIR` overrides the directory) |
| Redis `<prefix>:v1:{account-<kite user>}:order-budget` | order-rate budget, shared by all slots trading that Kite account |

Production uses `kite-prod` (`config/portfolio-production.json`), paper uses `kite-dev` and the pseudo-account
`PAPER`, so the two never share a lock or budget. `native-portfolio-validate` prints each slot's names.

