# WVAD — Williams Variable A/D Pressure strategy reference

Rust port of the TradingView indicator **Williams Variable A/D Pressure [MarkitTick]**
(Pine v6), run on **CRUDEOILM** 3-minute bars as an intraday MIS strategy through the generic
bar-model runner (the one Precision Sniper used before it was parked in `backup/`).

Written for kite-node 2.28.0 / wvad 1.1.0 (the active strategy; Sniper and SATS are parked).

**Licence.** The Pine source is © MarkitTick under CC BY-NC-SA 4.0
(https://creativecommons.org/licenses/by-nc-sa/4.0/). `crates/wvad` is a derivative work
under the same licence: keep the attribution, non-commercial use only, share-alike.

| Item | Where |
|---|---|
| Signal and trade model (pure, no I/O) | `crates/wvad` (`engine.rs`, `params.rs`, `filters.rs`) |
| Pine-exact moving averages and filters | `crates/wvad/src/filters.rs`; ATR and ADX in `crates/wvad/src/nt.rs` (`nautilus-indicators`) |
| Hook-up to the shared runner | `apps/kite-node/src/native_node/wvad_model.rs` |
| Slot settings, live strategy, runner, backtest | `slot_config.rs`, `model_strategy.rs`, `model_live.rs`, `model_backtest.rs` |
| Config | `config/wvad-crudeoilm.json` |
| Portfolio slot | `crudeoilm-wvad-202610` (disabled, live orders off: paper only) |
| Scripts | `deploy/run-wvad-paper.sh`, `deploy/run-wvad-live.sh` |

---

## 1. The indicator

Per bar:

* **raw WVAD** = (close − open) / (high − low) × volume, 0 when high = low
  (buying or selling pressure inside the bar, weighted by volume);
* **WVAD** = sum of raw WVAD over `wvad_len` bars (20), then the **adaptive filter**
  (SMA 9 by default; also None, EMA, RMA, Double WMA, Triple VWMA, HMA, LLAMA, Kalman);
* **signal line** = SMA of WVAD over `signal_len` (9); **histogram** = WVAD − signal.

**Signal**: WVAD crosses **above** the signal line → long; crosses **below** → short. Optional
filters (both off in the script's defaults and in the shipped config):

* **ADX**: ADX(`adx_len`) of the crossover bar ≥ `adx_threshold`;
* **HTF**: the last completed higher-timeframe candle (`htf_minutes`, buckets from the 09:00
  IST open) must be green for a long, red for a short.

## 2. Trade model

| Step | Rule |
|---|---|
| Entry | At the **crossover bar's close** |
| Stop | entry ∓ ATR(`atr_len`) × `sl_mult` (14, 1.5). Fixed: it never moves (`step_stop` off) |
| Targets | entry ± 1R / 2R / 3R (R = the stop distance) |
| TP1 / TP2 | With 3 lots: 1 lot out at each. The script only marks them |
| Exit | Stop, TP3 (the rest), an accepted **opposite** signal (one flip order), or the 23:15 square-off |
| Same-direction signal while in a trade | Ignored: the position keeps its levels (the script would redraw them) |
| Bar with both stop and a target | Stop first (counted as ambiguous) |

Live, every bid/ask quote is checked against the stop and targets (bid for a long, ask for a
short) and a target needs `tp_through_ticks` (1) beyond the level.

### 2.1 Differences from the script

| Point | Script | This port |
|---|---|---|
| Signal timing | `crossover(wvad[1], signal[1])`: drawn and alerted one bar after the cross, levels from `close[1]` / `atr[1]` | Acts at the cross bar's close: **same levels**, order one bar earlier than the script's alert |
| Same-direction signal (only possible with a filter on) | Redraws the levels | Keeps the open trade |
| TP1 / TP2 | Marks only | Close `tp1_lots` / `tp2_lots` |
| Session | None (positions run overnight) | Entries 09:00–23:00, square-off 23:15 |
| Stop gap | Fills at the stop | A bar opening through the stop fills at its open |
| ATR / ADX seeding | Pine RMA seeds with an SMA | Nautilus seeds with the first value; the 200-bar warm-up makes the gap negligible |
| Lock Signal, dashboard, colours, alert text | — | Not ported |

## 3. Settings

### 3.1 Slot

| Key | Shipped | Meaning |
|---|---|---|
| `strategy` | wvad | Must be "wvad" |
| `symbol`, `instrument_token` | CRUDEOILM26OCTFUT, 145894663 | Contract (roll before 19 Oct) |
| `bar_minutes` | 3 | Must equal `params.timeframe_minutes` |
| `lots`, `tp1_lots`, `tp2_lots` | 3, 1, 1 | TP1 and TP2 close one lot each, the last lot runs to TP3 / stop |
| `point_value` | 10 | ₹ per point per lot |
| `round_trip_cost_points`, `brokerage_per_order`, `slippage_points_per_side` | 2.0, 20, 1.0 | Backtest costs |
| `entries_until`, `square_off`, `entry_blackouts` | 23:00, 23:15, none | No entries on signal bars closing at/after 23:00 IST or inside a blackout; flatten on the 23:15 bar |

### 3.2 `params` (the Pine inputs)

| Key | Shipped (= script default) | Meaning |
|---|---|---|
| `wvad_len` | 20 | WVAD period (summation length) |
| `signal_len` | 9 | Signal line SMA |
| `adapt_filter`, `adapt_filter_len` | "SMA", 9 | "None", "SMA", "EMA", "RMA", "Double WMA", "Triple VWMA", "HMA", "LLAMA", "Kalman Filter" |
| `use_adx`, `adx_len`, `adx_threshold` | false, 14, 20 | ADX filter |
| `use_htf`, `htf_minutes` | false, 60 | Higher-timeframe candle filter |
| `atr_len`, `sl_mult` | 14, 1.5 | Stop distance |
| `tp1_r`, `tp2_r`, `tp3_r` | 1, 2, 3 | Targets in R |
| `step_stop` | false | Not in the script. On: TP1 → stop to entry, TP2 → stop to TP1 |
| `tick_size`, `min_risk_ticks` | 1, 1 | Trades with a smaller stop distance are skipped |
| `tp_through_ticks` | 1 | Live config. 0 = the script's touch rule |
| `warmup_bars` | 200 | Bars before the first signal may be taken |

## 4. Backtest, 19 Sep – 9 Oct 2026 (October contract, 14 days)

| Run | Trades | Win % | Net | PF | Max DD | Model R |
|---|---|---|---|---|---|---|
| Shipped config (3 lots, costs, slippage) | 257 | 37.7 | **−₹45,072** | 0.65 | ₹55,643 | −4.82 |
| Script-like (1 lot, no costs, touch, no session cut) | 267 | 34.5 | −₹1,698 | 0.96 | ₹9,911 | −8.51 |

Script-like in R: 92 W / 174 L, expectancy −0.03R, avg win 1.51R / loss 0.84R, 26 TP3, 134 SL,
95 reversals. About 19 trades a day on 3-minute bars: costs (2 pts × 3 lots + ₹20 per order +
slippage) take most of the edge. The ADX and HTF filters reduce the count.

## 5. Commands

| What | Command |
|---|---|
| Backtest | `./target/release/kite-node native-wvad-backtest config/wvad-crudeoilm.json 2026-09-19 2026-10-09` |
| Paper run | `deploy/run-wvad-paper.sh` |
| Live preflight only | `./target/release/kite-node native-wvad-live-preflight config/portfolio-production.json crudeoilm-wvad-202610 config/kite-production.json` |
| Live run | `deploy/run-wvad-live.sh` (needs the slot enabled with live orders; both are off now) |

Logs use the `wvad_` prefix (`wvad_signal`, `wvad_order`, `wvad_bar`, ...); results go to
`backtest_results/wvad/<symbol>/<time>/`.
