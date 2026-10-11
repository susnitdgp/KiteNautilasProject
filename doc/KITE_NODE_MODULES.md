# kite-node: what each source file does

Map of `apps/kite-node/src` (22 Rust files). Written for kite-node 2.29.0, where WVAD is the
only strategy kite-node runs (Precision Sniper and SATS runners are parked in `backup/`).

The Kite connection itself is not in kite-node: login, REST, WebSocket, order placement,
order-update stream, reconciliation and the paper-trading simulator are in
`crates/kite-adapter`; the shared order-rate budget is in `crates/kite-execution`; the WVAD
model is `crates/wvad`.

## Entry point

| File | Lines | What it does |
|---|---|---|
| `main.rs` | 12 | Starts the program, sets up TLS, hands the command line to `cli.rs` |
| `native_node/mod.rs` | 22 | Module list |
| `native_node/cli.rs` | 26 | Command → code: `native-wvad-backtest / -paper / -live-preflight / -live`, `native-portfolio-validate`, `native-kite-auth`, `native-kite-auth-login-url`, `native-kite-margins-check` |

## Strategy: shared runner and WVAD

| File | Lines | What it does |
|---|---|---|
| `bar_model.rs` | 47 | The interface every candle-by-candle model provides: process a bar, close a trade, mark a target, open-trade levels, dashboard rows and chart series, log fields, reject counts |
| `wvad_model.rs` | 161 | Plugs the `wvad` crate into that interface: names, warm-up size, title, dashboard rows, WVAD/signal chart, reject reasons; shipped-config test |
| `slot_config.rs` | 173 | The strategy JSON (contract, lots, TP1/TP2 lots, costs, entry window, blackouts, square-off) plus the strategy's own `params`; loads and validates it; the `ModelParams` trait a strategy's params implement |
| `model_strategy.rs` | 704 | **Live trading logic.** Model signals → target position → one market order at a time (a reversal is one flip order). Stop and targets checked on every bid/ask. 23:15 close-out, halt and flatten, broker cancels / rejections, 30 s order timeout. Writes the `wvad_*` log lines and the dashboard updates (events, trades, chart) |
| `model_live.rs` | 380 | **Runs one paper or live day.** Checks slot, session calendar, contract expiry and the Kite instrument master; downloads the warm-up history; checks the live-order gates; takes the slot lock file; builds the Nautilus node with the Kite data client and the real (production) or simulated (mock) order client; watches Ctrl+C / SIGTERM / SIGHUP / faults and flattens before stopping. Also the read-only live preflight |
| `model_backtest.rs` | 426 | **Backtest.** Replays Kite history through the same model with lots, slippage, costs and the daily close-out; writes `trades.csv`, `summary.json`, `summary.txt` under `backtest_results/<strategy>/<symbol>/<run>/` |
| `backtest_report.rs` | 10 | Writes the immutable per-run result folder |

## Market data: live candles from Kite

| File | Lines | What it does |
|---|---|---|
| `data.rs` | 505 | The Nautilus data client: Kite WebSocket ticks → quotes (bid/ask) and finished candles to the strategy; reconnects, backfills; `latency_bar` log lines |
| `ws_candles.rs` | 266 | Builds 3-minute candles from Kite full-mode ticks on exchange time; a gap or reconnect fails closed (flatten and stop) |
| `live_bars.rs` | 310 | Picks finalised candles from Kite history for the warm-up, detects gaps and revised candles, converts them to Nautilus bars |
| `live_backfill.rs` | 129 | Fetches the candle that was in progress at start-up (or across a feed gap) from Kite history once it is final, so the model sees no hole |
| `bar_timing.rs` | 7 | How long after a candle's close Kite's history is treated as final |
| `session_calendar.rs` | 172 | MCX trading days, holidays and special sessions from `config/mcx-session-calendar.json` (IST) |
| `status.rs` | 44 | Small "feed status" custom data type the data client publishes inside Nautilus |

## Portfolio and run control

| File | Lines | What it does |
|---|---|---|
| `portfolio.rs` | 293 | Reads and validates `config/portfolio-production.json` (slot IDs, tokens, rollover, live-order gate, Redis key names); the `native-portfolio-validate` report |
| `live_control.rs` | 95 | Flags shared by the strategy, the data feed and the runner's watcher: stop requested, fault, position flat, done |

## Dashboard

| File | Lines | What it does |
|---|---|---|
| `dashboard.rs` | 460 | The dashboard's data (`Board`): status, price, model panel and chart, position, realised / unrealised, trades table, events; the end-of-run text summary; the JSON log writer (`emit`) and console notes (`note`) |
| `dash_writer.rs` | 404 | Publishes the board to the dashboard Redis (database 1) from its own thread, fed by a lock-free queue that never blocks trading (full queue → update dropped and counted) |

The web page itself is `tools/dashboard/app.py` (Streamlit, read-only); see
`doc/LIVE_DASHBOARD.md`.

## Tests only

| File | Lines | What it does |
|---|---|---|
| `synthetic.rs` | 55 | Synthetic market data for offline tests |
| `test_support.rs` | 38 | Test helpers (session bounds, sample candles) |

## How a live day flows through the files

1. `main.rs` → `cli.rs` → `model_live::run::<wvad::Params>`.
2. `model_live.rs` loads the slot (`portfolio.rs`, `slot_config.rs`, `session_calendar.rs`), downloads
   the warm-up (`live_bars.rs`), takes the lock and builds the node.
3. `data.rs` + `ws_candles.rs` (+ `live_backfill.rs`) deliver quotes and 3-minute candles.
4. `model_strategy.rs` feeds candles to the model (`bar_model.rs` → `wvad_model.rs` → `crates/wvad`),
   checks every quote against stop / targets, and sends orders through the kite-adapter order client.
5. `dashboard.rs` / `dash_writer.rs` publish the live state; JSON log lines go to `logs/`.
6. At 23:15 the strategy flattens; `model_live.rs` waits for flat, stops the node and prints the summary.
