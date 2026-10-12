# Lorentzian Classification (Rust port) — backtest only

`crates/lorentzian` (v1.0.0) ports **Machine Learning: Lorentzian Classification v2.0** (© jdehorty,
MPL 2.0) with its libraries MLExtensions v2 and KernelFunctions v2. It plugs into the shared bar-model
runner like WVAD. Only the backtest command is wired; there is no paper or live command.

    ./target/release/kite-node native-lorentzian-backtest config/lorentzian-crudeoilm.json 2026-09-21 2026-10-09

Results go to `backtest_results/lorentzian/<symbol>/<run>/` (`summary.txt`, `summary.json`, `trades.csv`).

## Model (per confirmed bar)

1. **Features** (defaults): RSI 14, WaveTrend 10/11, CCI 20, ADX 20, RSI 9, each normalised as
   MLExtensions does (RSI / ADX rescaled 0..100 → 0..1; CCI / WT by running min / max).
2. **Label**: close 4 bars ago vs now, stored with the current bar's features. As in the script, a
   rise is labelled -1 (short) and a fall +1, so the model effectively learns "the last 4 bars reverse".
3. **Neighbours**: Lorentzian distance `Σ ln(1 + |Δf|)`, bars with index % 4 == 0 skipped, a candidate
   kept when its distance is >= the running threshold; `neighbors_count` (8) kept, the threshold reset
   to the distance at ¾ of the list. The neighbour lists persist across bars, as the script's `var`
   arrays. **Prediction** = sum of the labels (-8..8).
4. **Filters**: volatility ATR(1) > ATR(10), regime (KLMF slope decline >= -0.1), optional ADX(14) > 20.
   Signal = sign of the prediction when the filters pass, else the previous signal.
5. **Entry**: a signal change (long / short) when the kernel slope agrees (rational quadratic, h 8,
   r 8, x 25) and the optional EMA / SMA 200 filters agree. Entry at the signal bar's close.
6. **Exit** (`exit_mode`):
   * `Fixed` (script default): close at the 4th bar after entry. With `exit_on_signal_flip` (on),
     close earlier when the model signal flips against the trade (the script's trade stats count that
     as a stop-loss; the script itself then shows no exit at all).
   * `Dynamic` (script "Use Dynamic Exits"): close on the first kernel slope turn against the trade.
   * Always: an accepted opposite entry reverses; an ATR(14) × `sl_mult` (2.0) safety stop, checked
     first on bar OHLC (not in the script); the runner's 23:00 entry cut-off and 23:15 square-off.
7. No take-profit levels: `tp1_lots` / `tp2_lots` must be 0 (the model refuses partials). TP columns
   in `trades.csv` are `NaN`. Grade: High when |prediction| >= 6 of 8, Mid >= 4, else Low.

## Differences from TradingView (why the Rust signals won't match the chart bar for bar)

* **Window**: the script sets its neighbour window from `last_bar_index` (the chart's final bar), so its
  signals change with how much history is loaded, and on a long chart its loop runs backwards over old
  bars and skips the latest ~2000. The port uses a rolling window of the last `max_bars_back` bars,
  oldest first — deterministic and usable live.
* **Normalisation start**: CCI / WT use min / max since the model started (the script: since the
  chart's first bar).
* The **trade-stats table** on the chart (win rate etc.) is not a backtest: it counts wins from
  intrabar highs / lows with no costs. Use this backtest's costed numbers.

## Warm-up and data

`warmup_bars` 2000 (the full window) and `history_days` 21 calendar days fetched before FROM. Signal
changes before the model is warm are counted as rejected "Data not ready / warmup". Kite history of a
monthly contract starts at its listing, so the earliest usable FROM is about 12 trading days after it.
