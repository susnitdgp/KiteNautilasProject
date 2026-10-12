//! Lorentzian Classification model, bar for bar on confirmed bars:
//! features → label → nearest neighbours → prediction → filters → signal → kernel →
//! (open trade: stop first, reversal, 4-bar / flip / kernel exit) → entry at the signal bar's
//! close.
use crate::params::{ExitMode, Feature, FeatureKind, HOLD_BARS, Params};
use crate::ta::{Cci, ExpAvg, JAdx, Normalize, Regime, Rsi, Sma, TrueRange, WaveTrend, gaussian, rational_quadratic, rescale};
use serde::Serialize;
use std::collections::VecDeque;
use wvad::{Bar, Event};

/// One feature series (`series_from` in the script).
#[derive(Debug, Clone)]
enum Calc {
    Rsi(Rsi, ExpAvg),
    Wt(WaveTrend),
    Cci(Cci, ExpAvg, Normalize),
    Adx(JAdx),
}
impl Calc {
    fn new(f: Feature) -> Self {
        match f.kind {
            FeatureKind::Rsi => Calc::Rsi(Rsi::new(f.a), ExpAvg::ema(f.b)),
            FeatureKind::Wt => Calc::Wt(WaveTrend::new(f.a, f.b)),
            FeatureKind::Cci => Calc::Cci(Cci::new(f.a), ExpAvg::ema(f.b), Normalize::default()),
            FeatureKind::Adx => Calc::Adx(JAdx::new(f.a)),
        }
    }
    /// `n_rsi(close)`, `n_wt(hlc3)`, `n_cci(close)`, `n_adx(high, low, close)`.
    fn update(&mut self, b: &Bar) -> f64 {
        match self {
            Calc::Rsi(rsi, ema) => rescale(ema.update(rsi.update(b.close)), 0.0, 100.0),
            Calc::Wt(wt) => wt.update((b.high + b.low + b.close) / 3.0),
            Calc::Cci(cci, ema, norm) => norm.update(ema.update(cci.update(b.close))),
            Calc::Adx(adx) => rescale(adx.update(b.high, b.low, b.close), 0.0, 100.0),
        }
    }
}

/// `get_lorentzian_distance`: Σ ln(1 + |Δf|) over the first `n` features (`na` → NaN).
fn lorentzian_distance(a: &[f64; 5], b: &[f64; 5], n: usize) -> f64 {
    (0..n).map(|k| (1.0 + (a[k] - b[k]).abs()).ln()).sum()
}

#[derive(Clone, Debug)]
pub struct Trade {
    pub dir: i32,
    pub entry_bar: u64,
    pub entry: f64,
    pub stop: f64,
    pub risk: f64,
}
impl Trade {
    fn finish(self, px: f64, reason: &'static str) -> Event {
        let gross_r = self.dir as f64 * (px - self.entry) / self.risk;
        Event::Exit { dir: self.dir, price: px, reason, fraction: 1.0, gross_r, ambiguous: false }
    }
}

/// Why a signal change was not taken (diagnostics).
pub fn reject_reason(code: u8) -> &'static str {
    match code {
        1 => "Data not ready / warmup",
        2 => "Outside entry window",
        3 => "Kernel against",
        4 => "EMA/SMA trend against",
        9 => "Invalid risk (ATR stop below minimum)",
        12 => "Same direction already open",
        _ => "Accepted",
    }
}

/// Model state after the latest bar (dashboard / logs).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub ready: bool,
    /// Sum of the neighbours' labels, -k..k.
    pub prediction: f64,
    /// Filtered signal: 1 long, -1 short, 0 none yet.
    pub signal: i32,
    /// Neighbours currently held.
    pub neighbours: usize,
    /// Kernel estimate (rational quadratic) and its slope: 1 rising, -1 falling, 0 flat / n.a.
    pub kernel: f64,
    pub kernel_slope: i32,
    /// Volatility / regime / ADX filters all pass.
    pub filters: bool,
    /// Regime value (normalised slope decline) and jdehorty ADX(14).
    pub regime: f64,
    pub adx: f64,
    pub atr: f64,
}

#[derive(Debug)]
pub struct Engine {
    p: Params,
    status: Status,
    bar_index: u64,
    calcs: Vec<Calc>,
    /// Rolling neighbour window: (bar index, features, label), oldest first.
    window: VecDeque<(u64, [f64; 5], i32)>,
    /// The script's `var` arrays: kept across bars.
    distances: VecDeque<f64>,
    predictions: VecDeque<i32>,
    closes: VecDeque<f64>,
    closes_cap: usize,
    signal: i32,
    /// Last three kernel estimates (NaN = not yet).
    yhat1: VecDeque<f64>,
    ema: ExpAvg,
    sma: Sma,
    tr: TrueRange,
    atr1: ExpAvg,
    atr10: ExpAvg,
    regime: Regime,
    adx: JAdx,
    atr: ExpAvg,
    pub trade: Option<Trade>,
    /// Reject-code counts for signal changes not taken.
    pub rejects: [u32; 13],
    /// Signal changes seen after the warm-up.
    pub candidates: u32,
}

impl Engine {
    pub fn new(p: Params) -> Self {
        let closes_cap = (p.kernel_regression_level + 2).max(5);
        Self {
            status: Status::default(),
            bar_index: 0,
            calcs: p.features.iter().map(|f| Calc::new(*f)).collect(),
            window: VecDeque::with_capacity(p.max_bars_back + 1),
            distances: VecDeque::new(),
            predictions: VecDeque::new(),
            closes: VecDeque::with_capacity(closes_cap + 1),
            closes_cap,
            signal: 0,
            yhat1: VecDeque::from(vec![f64::NAN; 3]),
            ema: ExpAvg::ema(p.ema_period),
            sma: Sma::new(p.sma_period),
            tr: TrueRange::default(),
            atr1: ExpAvg::rma(1),
            atr10: ExpAvg::rma(10),
            regime: Regime::default(),
            adx: JAdx::new(14),
            atr: ExpAvg::rma(p.atr_len),
            trade: None,
            rejects: [0; 13],
            candidates: 0,
            p,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn params(&self) -> &Params {
        &self.p
    }

    /// Closes the open model trade at `price` (live stop on a tick, or the daily square-off
    /// the script does not have).
    pub fn force_close(&mut self, price: f64, reason: &'static str) -> Option<Event> {
        Some(self.trade.take()?.finish(price, reason))
    }

    /// Nearest-neighbour step for the current bar (the script's core ML loop), over the
    /// rolling window, oldest first. Returns the prediction.
    fn predict(&mut self, f: &[f64; 5]) -> f64 {
        let k = self.p.neighbors_count;
        let fc = self.p.feature_count;
        let cut = ((k as f64) * 3.0 / 4.0).round() as usize;
        let mut last = -1.0_f64;
        for (i, fi, yi) in &self.window {
            if i % 4 == 0 {
                continue;
            }
            let d = lorentzian_distance(f, fi, fc);
            if d >= last {
                last = d;
                self.distances.push_back(d);
                self.predictions.push_back(*yi);
                if self.predictions.len() > k {
                    last = self.distances[cut];
                    self.distances.pop_front();
                    self.predictions.pop_front();
                }
            }
        }
        self.predictions.iter().map(|v| f64::from(*v)).sum()
    }

    /// Feed one confirmed bar. `entries_allowed` = false: signals are not taken (the open
    /// trade is still managed).
    pub fn on_bar(&mut self, b: Bar, entries_allowed: bool) -> Vec<Event> {
        let p = self.p.clone();
        let idx = self.bar_index;
        self.bar_index += 1;

        // ── features and training label (close 4 bars ago vs now) ──
        let mut f = [f64::NAN; 5];
        for (k, c) in self.calcs.iter_mut().enumerate() {
            f[k] = c.update(&b);
        }
        self.closes.push_back(b.close);
        if self.closes.len() > self.closes_cap {
            self.closes.pop_front();
        }
        let n = self.closes.len();
        let label = if n >= 5 {
            let c4 = self.closes[n - 5];
            if c4 < b.close {
                -1
            } else if c4 > b.close {
                1
            } else {
                0
            }
        } else {
            0
        };
        self.window.push_back((idx, f, label));
        if self.window.len() > p.max_bars_back {
            self.window.pop_front();
        }
        let prediction = self.predict(&f);

        // ── filters ──
        let tr = self.tr.update(b.high, b.low, b.close);
        let (atr1, atr10) = (self.atr1.update(tr), self.atr10.update(tr));
        let atr = self.atr.update(tr);
        let regime = self.regime.update((b.open + b.high + b.low + b.close) / 4.0, b.high, b.low);
        let adx = self.adx.update(b.high, b.low, b.close);
        let vol_ok = !p.use_volatility_filter || atr1 > atr10;
        let regime_ok = !p.use_regime_filter || regime >= p.regime_threshold;
        let adx_ok = !p.use_adx_filter || adx > p.adx_threshold;
        let filters = vol_ok && regime_ok && adx_ok;

        // ── signal ──
        let prev_signal = self.signal;
        if prediction > 0.0 && filters {
            self.signal = 1;
        } else if prediction < 0.0 && filters {
            self.signal = -1;
        }
        let changed = idx > 0 && self.signal != prev_signal;

        let ema = self.ema.update(b.close);
        let sma = self.sma.update(b.close);
        let trend_ok = |d: i32| {
            let ema_ok = !p.use_ema_filter || if d == 1 { b.close > ema } else { b.close < ema };
            let sma_ok = !p.use_sma_filter || if d == 1 { b.close > sma } else { b.close < sma };
            ema_ok && sma_ok
        };

        // ── kernel regression ──
        let y1 = rational_quadratic(&self.closes, p.kernel_lookback, p.kernel_relative_weight, p.kernel_regression_level).unwrap_or(f64::NAN);
        let y2 = gaussian(&self.closes, p.kernel_lookback - p.kernel_lag, p.kernel_regression_level).unwrap_or(f64::NAN);
        self.yhat1.push_back(y1);
        self.yhat1.pop_front();
        let (k2, k1, k0) = (self.yhat1[0], self.yhat1[1], self.yhat1[2]);
        let bullish_rate = k1 < k0;
        let bearish_rate = k1 > k0;
        let bullish_change = bullish_rate && k2 > k1;
        let bearish_change = bearish_rate && k2 < k1;
        let kernel_ok = |d: i32| {
            !p.use_kernel_filter
                || if p.use_kernel_smoothing {
                    if d == 1 { y2 >= y1 } else { y2 <= y1 }
                } else if d == 1 {
                    bullish_rate
                } else {
                    bearish_rate
                }
        };

        let ready = idx >= p.warmup_bars as u64 && atr.is_finite() && atr > 0.0 && k0.is_finite() && k1.is_finite();
        self.status = Status {
            ready,
            prediction,
            signal: self.signal,
            neighbours: self.predictions.len(),
            kernel: k0,
            kernel_slope: if bullish_rate {
                1
            } else if bearish_rate {
                -1
            } else {
                0
            },
            filters,
            regime,
            adx,
            atr,
        };

        // ── signal change → accepted entry direction ──
        let risk = atr * p.sl_mult;
        let tick = p.tick_size;
        let mut signal_dir = 0;
        if changed && self.signal != 0 && idx >= p.warmup_bars as u64 {
            let d = self.signal;
            self.candidates += 1;
            let code = if !ready {
                1
            } else if !entries_allowed {
                2
            } else if !trend_ok(d) {
                4
            } else if !kernel_ok(d) {
                3
            } else if !(risk.is_finite() && risk >= f64::from(p.min_risk_ticks) * tick - tick * 1e-6) || b.close - risk <= 0.0 {
                9
            } else {
                0
            };
            if code == 0 {
                signal_dir = d;
            } else {
                self.rejects[code as usize] += 1;
            }
        }

        let mut events = Vec::new();
        // ── open trade: stop first, reversal, then the exit rule ──
        if let Some(t) = self.trade.as_ref()
            && idx > t.entry_bar
        {
            let stopped = if t.dir == 1 { b.low <= t.stop } else { b.high >= t.stop };
            let exit: Option<(f64, &'static str)> = if stopped {
                // a gap through the stop fills at the open
                Some((if t.dir == 1 { b.open.min(t.stop) } else { b.open.max(t.stop) }, "SL"))
            } else if signal_dir == -t.dir {
                Some((b.close, "Reversal"))
            } else {
                match p.exit_mode {
                    ExitMode::Fixed if idx - t.entry_bar >= HOLD_BARS => Some((b.close, "Hold 4 bars")),
                    ExitMode::Fixed if p.exit_on_signal_flip && self.signal == -t.dir => Some((b.close, "Signal flip")),
                    ExitMode::Dynamic if (t.dir == 1 && bearish_change) || (t.dir == -1 && bullish_change) => {
                        Some((b.close, "Kernel exit"))
                    }
                    _ => None,
                }
            };
            if let Some((px, reason)) = exit {
                let t = self.trade.take().expect("open trade");
                events.push(t.finish(px, reason));
            }
        }

        // ── entry at the signal bar's close ──
        if signal_dir != 0 {
            if self.trade.is_some() {
                self.rejects[12] += 1;
            } else {
                let e = b.close;
                let stop = e - signal_dir as f64 * risk;
                let k = p.neighbors_count as f64;
                let strength = prediction.abs() / k;
                let grade = if strength >= 0.75 {
                    "High"
                } else if strength >= 0.5 {
                    "Mid"
                } else {
                    "Low"
                };
                self.trade = Some(Trade { dir: signal_dir, entry_bar: idx, entry: e, stop, risk });
                events.push(Event::Entry {
                    dir: signal_dir,
                    price: e,
                    stop,
                    // no take-profit levels: the model exits on time / kernel / stop
                    tp1: f64::NAN,
                    tp2: f64::NAN,
                    tp3: f64::NAN,
                    score: prediction.abs(),
                    score_max: k,
                    grade,
                    origin: "ANN",
                });
            }
        }
        events
    }
}
