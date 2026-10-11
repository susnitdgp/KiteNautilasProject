//! Williams Variable A/D Pressure model, bar for bar on confirmed bars:
//! indicators → crossover → filters → (open trade: stop first, targets, reversal) → entry
//! at the crossover bar's close.
use crate::{
    filters::{Adaptive, Highest, Sma},
    params::Params,
};
use serde::Serialize;
use sniper::{
    Bar, Event,
    nt::{Atr, Dmi},
};

const IST_OFFSET: i64 = 19_800;
/// MCX session open, seconds after IST midnight: higher-timeframe buckets start here.
const SESSION_OPEN_SECS: i64 = 9 * 3600;
/// Window of the script's "Strength" (|WVAD| / its 100-bar high).
const STRENGTH_LEN: usize = 100;
const VOLUME_MEAN_LEN: usize = 20;

#[derive(Clone, Debug)]
pub struct Trade {
    pub dir: i32,
    pub entry_bar: u64,
    pub entry: f64,
    pub initial_stop: f64,
    pub stop: f64,
    pub risk: f64,
    pub tp1: f64,
    pub tp2: f64,
    pub tp3: f64,
    pub hit1: bool,
    pub hit2: bool,
    pub hit3: bool,
    /// The bar that stopped the trade also touched a target (stop taken first).
    pub ambiguous: bool,
    /// Open fraction of the original position.
    pub remaining: f64,
    /// Realised gross R so far (partials).
    pub gross_r: f64,
}

impl Trade {
    fn take(&mut self, px: f64, fraction: f64) -> f64 {
        let f = fraction.clamp(0.0, self.remaining);
        self.gross_r += f * self.dir as f64 * (px - self.entry) / self.risk;
        self.remaining = (self.remaining - f).max(0.0);
        f
    }
    fn finish(mut self, px: f64, reason: &'static str) -> Event {
        let fraction = self.remaining;
        self.take(px, fraction);
        Event::Exit { dir: self.dir, price: px, reason, fraction, gross_r: self.gross_r, ambiguous: self.ambiguous }
    }
    /// Target `level` touched: TP1 / TP2 close their fraction (if any); TP3 is only marked
    /// here (the caller closes the rest).
    fn hit(&mut self, level: u8, p: &Params) -> Option<Event> {
        let (price, fraction) = match level {
            1 if !self.hit1 => {
                self.hit1 = true;
                (self.tp1, p.tp1_close_fraction)
            }
            2 if !self.hit2 => {
                self.hit2 = true;
                (self.tp2, p.tp2_close_fraction)
            }
            3 if !self.hit3 => {
                self.hit3 = true;
                return None;
            }
            _ => return None,
        };
        if fraction <= 0.0 {
            return None;
        }
        let f = self.take(price, fraction);
        (f > 0.0).then_some(Event::Partial { dir: self.dir, level, price, fraction: f })
    }
}

/// Why a crossover was not taken (diagnostics).
pub fn reject_reason(code: u8) -> &'static str {
    match code {
        1 => "Data not ready / warmup",
        2 => "Outside entry window",
        3 => "ADX below threshold",
        4 => "HTF candle against",
        9 => "Invalid risk (ATR stop below minimum)",
        12 => "Same direction already open",
        _ => "Accepted",
    }
}

/// Model state after the latest bar (dashboard / logs).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub ready: bool,
    /// 1 WVAD above its signal line, -1 below, 0 equal / not ready (the script's "Bias").
    pub bias: i32,
    pub wvad: f64,
    pub signal: f64,
    pub hist: f64,
    /// |WVAD| / its 100-bar high, 0..1 (the script's "Strength").
    pub strength: f64,
    /// Volume / its 20-bar mean.
    pub vol_ratio: f64,
    pub adx: f64,
    pub atr: f64,
    /// Last completed higher-timeframe candle: 1 green, -1 red, 0 flat; None if off / not yet.
    pub htf: Option<i32>,
}

/// Higher-timeframe candles built from the chart bars, aligned to the 09:00 IST open.
#[derive(Debug, Default)]
struct HtfCandles {
    /// (day, bucket) of the candle being built, its open and latest close.
    current: Option<((i64, i64), f64, f64)>,
    /// Last completed candle: (open, close).
    done: Option<(f64, f64)>,
}
impl HtfCandles {
    fn update(&mut self, b: &Bar, bar_secs: i64, htf_secs: i64) {
        let ist = b.start + IST_OFFSET;
        let (day, sod) = (ist.div_euclid(86_400), ist.rem_euclid(86_400));
        let bucket = (sod - SESSION_OPEN_SECS).div_euclid(htf_secs);
        let key = (day, bucket);
        match &mut self.current {
            Some((k, _, close)) if *k == key => *close = b.close,
            _ => {
                if let Some((_, o, c)) = self.current.take() {
                    self.done = Some((o, c));
                }
                self.current = Some((key, b.open, b.close));
            }
        }
        let bucket_end = SESSION_OPEN_SECS + (bucket + 1) * htf_secs;
        if sod + bar_secs >= bucket_end
            && let Some((_, o, c)) = self.current.take()
        {
            self.done = Some((o, c));
        }
    }
    fn direction(&self) -> Option<i32> {
        self.done.map(|(o, c)| if c > o { 1 } else if c < o { -1 } else { 0 })
    }
}

#[derive(Debug)]
pub struct Engine {
    p: Params,
    status: Status,
    bar_index: u64,
    sum: Sma,
    filter: Adaptive,
    signal: Sma,
    atr: Atr,
    dmi: Dmi,
    vol_mean: Sma,
    strength_max: Highest,
    htf: HtfCandles,
    /// (WVAD, signal) of the previous bar, when both existed.
    prev: Option<(f64, f64)>,
    pub trade: Option<Trade>,
    /// Reject-code counts for crossovers not taken.
    pub rejects: [u32; 13],
    /// Raw crossovers seen after the warm-up.
    pub candidates: u32,
}

impl Engine {
    pub fn new(p: Params) -> Self {
        Self {
            status: Status::default(),
            bar_index: 0,
            sum: Sma::new(p.wvad_len),
            filter: Adaptive::new(p.adapt_filter, p.adapt_filter_len),
            signal: Sma::new(p.signal_len),
            atr: Atr::new(p.atr_len),
            dmi: Dmi::new(p.adx_len, p.adx_len),
            vol_mean: Sma::new(VOLUME_MEAN_LEN),
            strength_max: Highest::new(STRENGTH_LEN),
            htf: HtfCandles::default(),
            prev: None,
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

    /// Closes the open model trade at `price` (live stop / TP3 on a tick, or a daily
    /// square-off the script does not have).
    pub fn force_close(&mut self, price: f64, reason: &'static str) -> Option<Event> {
        Some(self.trade.take()?.finish(price, reason))
    }

    /// Live: a tick reached target `level` (1..=3) before the bar closed. Applies the same
    /// partials the bar-close model would; TP3 closes the rest.
    pub fn mark_target(&mut self, level: u8) -> Vec<Event> {
        let p = self.p.clone();
        let Some(t) = self.trade.as_mut() else { return Vec::new() };
        let mut out = Vec::new();
        for l in 1..=level {
            if let Some(ev) = t.hit(l, &p) {
                out.push(ev);
            }
        }
        if level >= 3 {
            let tp3 = t.tp3;
            if let Some(ev) = self.force_close(tp3, "TP3") {
                out.push(ev);
            }
        }
        out
    }

    /// Feed one confirmed bar. `entries_allowed` = false: crossovers are not taken (the
    /// open trade is still managed).
    pub fn on_bar(&mut self, b: Bar, entries_allowed: bool) -> Vec<Event> {
        let p = self.p.clone();
        let idx = self.bar_index;
        self.bar_index += 1;

        // ── indicators (every bar) ──
        let raw = if b.high != b.low { (b.close - b.open) / (b.high - b.low) * b.volume } else { 0.0 };
        let sum = self.sum.update(raw).map(|m| m * p.wvad_len as f64);
        let wvad = self.filter.update(sum, b.volume);
        let signal = wvad.and_then(|w| self.signal.update(w));
        let atr = self.atr.update(b.high, b.low, b.close);
        let adx = self.dmi.update(b.high, b.low, b.close).map(|d| d.2);
        let vol_mean = self.vol_mean.update(b.volume);
        let strength = wvad.map(|w| {
            let max = self.strength_max.update(w.abs());
            if max > 0.0 { w.abs() / max } else { 0.0 }
        });
        if p.use_htf {
            self.htf.update(&b, i64::from(p.timeframe_minutes) * 60, i64::from(p.htf_minutes) * 60);
        }
        let htf = if p.use_htf { self.htf.direction() } else { None };

        let prev = self.prev;
        self.prev = wvad.zip(signal);
        let (cross_up, cross_down) = match (prev, wvad.zip(signal)) {
            (Some((pw, ps)), Some((w, s))) => (w > s && pw <= ps, w < s && pw >= ps),
            _ => (false, false),
        };
        let atr_v = atr.unwrap_or(0.0);
        let ready = idx >= p.warmup_bars as u64
            && signal.is_some()
            && atr_v > 0.0
            && (!p.use_adx || adx.is_some())
            && (!p.use_htf || htf.is_some());
        self.status = Status {
            ready,
            bias: match wvad.zip(signal) {
                Some((w, s)) if w > s => 1,
                Some((w, s)) if w < s => -1,
                _ => 0,
            },
            wvad: wvad.unwrap_or(f64::NAN),
            signal: signal.unwrap_or(f64::NAN),
            hist: wvad.zip(signal).map(|(w, s)| w - s).unwrap_or(f64::NAN),
            strength: strength.unwrap_or(0.0),
            vol_ratio: vol_mean.filter(|m| *m > 0.0).map(|m| b.volume / m).unwrap_or(0.0),
            adx: adx.unwrap_or(f64::NAN),
            atr: atr_v,
            htf,
        };

        // ── crossover → accepted signal (same-direction check comes after the exits) ──
        let risk = atr_v * p.sl_mult;
        let tick = p.tick_size;
        let mut signal_dir = 0;
        if (cross_up || cross_down) && idx >= p.warmup_bars as u64 {
            let d = if cross_up { 1 } else { -1 };
            self.candidates += 1;
            let code = if !ready {
                1
            } else if !entries_allowed {
                2
            } else if p.use_adx && adx.is_none_or(|a| a < p.adx_threshold) {
                3
            } else if p.use_htf && htf != Some(d) {
                4
            } else if risk < f64::from(p.min_risk_ticks) * tick - tick * 1e-6 || b.close - risk * p.tp3_r.max(1.0) <= 0.0 {
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
        // ── open trade: stop first, then targets (TP3 closes), reversal, optional step stop ──
        if let Some(t) = self.trade.as_mut()
            && idx > t.entry_bar
        {
            let d = t.dir as f64;
            let stopped = if t.dir == 1 { b.low <= t.stop } else { b.high >= t.stop };
            let through = f64::from(p.tp_through_ticks) * tick;
            let touch = |lvl: f64| if t.dir == 1 { b.high >= lvl + through } else { b.low <= lvl - through };
            let (t1, t2, t3) = (!t.hit1 && touch(t.tp1), !t.hit2 && touch(t.tp2), !t.hit3 && touch(t.tp3));
            let mut exit: Option<(f64, &'static str)> = None;
            if stopped {
                // a gap through the stop fills at the open
                let fill = if t.dir == 1 { b.open.min(t.stop) } else { b.open.max(t.stop) };
                t.ambiguous |= t1 || t2 || t3;
                exit = Some((fill, if t.stop == t.initial_stop { "SL" } else { "Step stop" }));
            } else {
                for (touched, level) in [(t1, 1u8), (t2, 2), (t3, 3)] {
                    if touched && let Some(ev) = t.hit(level, &p) {
                        events.push(ev);
                    }
                }
                if t3 {
                    exit = Some((t.tp3, "TP3"));
                } else if signal_dir == -t.dir {
                    exit = Some((b.close, "Reversal"));
                } else if p.step_stop {
                    let next = if t.hit2 { t.tp1 } else if t.hit1 { t.entry } else { t.stop };
                    if d * (next - t.stop) > tick * 0.1 {
                        t.stop = next;
                    }
                }
            }
            if let Some((px, reason)) = exit {
                let t = self.trade.take().expect("open trade");
                events.push(t.finish(px, reason));
            }
        }

        // ── entry at the crossover bar's close ──
        if signal_dir != 0 {
            if self.trade.is_some() {
                // the script would redraw its levels; the position keeps its own (never adds)
                self.rejects[12] += 1;
            } else {
                let e = b.close;
                let df = signal_dir as f64;
                let stop = e - df * risk;
                let tp = [p.tp1_r, p.tp2_r, p.tp3_r].map(|m| e + df * risk * m);
                let s = strength.unwrap_or(0.0);
                let grade = if s >= 0.66 { "High" } else if s >= 0.33 { "Mid" } else { "Low" };
                self.trade = Some(Trade {
                    dir: signal_dir,
                    entry_bar: idx,
                    entry: e,
                    initial_stop: stop,
                    stop,
                    risk,
                    tp1: tp[0],
                    tp2: tp[1],
                    tp3: tp[2],
                    hit1: false,
                    hit2: false,
                    hit3: false,
                    ambiguous: false,
                    remaining: 1.0,
                    gross_r: 0.0,
                });
                events.push(Event::Entry {
                    dir: signal_dir,
                    price: e,
                    stop,
                    tp1: tp[0],
                    tp2: tp[1],
                    tp3: tp[2],
                    score: (s * 100.0).round(),
                    score_max: 100.0,
                    grade,
                    origin: "ATR",
                });
            }
        }
        events
    }
}
