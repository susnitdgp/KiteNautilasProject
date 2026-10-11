//! Inputs of Williams Variable A/D Pressure [MarkitTick] that affect the trade model.
//! Visual, dashboard, colour and alert-text inputs are not ported; "Lock Signal" only acts
//! on the chart's last bar and is not ported either.
use serde::{Deserialize, Serialize};

/// "🧠 Adaptive Filter" applied to the WVAD sum before the signal line.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Filter {
    None,
    #[serde(rename = "SMA")]
    Sma,
    #[serde(rename = "EMA")]
    Ema,
    #[serde(rename = "RMA")]
    Rma,
    #[serde(rename = "Double WMA")]
    DoubleWma,
    #[serde(rename = "Triple VWMA")]
    TripleVwma,
    #[serde(rename = "HMA")]
    Hma,
    #[serde(rename = "LLAMA")]
    Llama,
    #[serde(rename = "Kalman Filter")]
    Kalman,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Chart timeframe, minutes (must equal the slot's `bar_minutes`; used for the HTF buckets).
    pub timeframe_minutes: u32,
    /// "WVAD Period": summation length.
    pub wvad_len: usize,
    /// "Signal Length": SMA of the filtered WVAD.
    pub signal_len: usize,
    /// "Adaptive Filter" and its length.
    pub adapt_filter: Filter,
    pub adapt_filter_len: usize,
    /// "Use ADX Filter": signals need ADX (of the crossover bar) >= `adx_threshold`.
    pub use_adx: bool,
    pub adx_len: usize,
    pub adx_threshold: f64,
    /// "Use HTF Confirmation": longs need the last completed higher-timeframe candle green
    /// (close > open), shorts red. Buckets start at the 09:00 IST session open.
    pub use_htf: bool,
    pub htf_minutes: u32,
    /// "ATR Length" and "SL × ATR".
    pub atr_len: usize,
    pub sl_mult: f64,
    /// "TP1/TP2/TP3 × R".
    pub tp1_r: f64,
    pub tp2_r: f64,
    pub tp3_r: f64,
    /// Fraction of the ORIGINAL position closed at TP1 / TP2. Set by the slot from
    /// `tp1_lots` / `tp2_lots`; 0 = whole position (the script: TP1/TP2 are only marks).
    pub tp1_close_fraction: f64,
    pub tp2_close_fraction: f64,
    /// Not in the script (its stop never moves). On: TP1 → stop to entry, TP2 → stop to
    /// TP1, effective from the next bar.
    pub step_stop: bool,
    pub tick_size: f64,
    /// Trades whose stop distance (ATR × `sl_mult`) is below this many ticks are skipped.
    pub min_risk_ticks: u32,
    /// A take-profit counts only when the bar trades this many ticks THROUGH it. 0 = touch,
    /// as the script.
    pub tp_through_ticks: u32,
    /// Bars before the first signal may be taken (indicators settle).
    pub warmup_bars: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            timeframe_minutes: 3,
            wvad_len: 20,
            signal_len: 9,
            adapt_filter: Filter::Sma,
            adapt_filter_len: 9,
            use_adx: false,
            adx_len: 14,
            adx_threshold: 20.0,
            use_htf: false,
            htf_minutes: 60,
            atr_len: 14,
            sl_mult: 1.5,
            tp1_r: 1.0,
            tp2_r: 2.0,
            tp3_r: 3.0,
            tp1_close_fraction: 0.0,
            tp2_close_fraction: 0.0,
            step_stop: false,
            tick_size: 1.0,
            min_risk_ticks: 1,
            tp_through_ticks: 0,
            warmup_bars: 200,
        }
    }
}

impl Params {
    /// Fewest bars after which every indicator the model uses has a value.
    pub fn min_warmup(&self) -> usize {
        let filter = match self.adapt_filter {
            Filter::None | Filter::Kalman => 0,
            Filter::DoubleWma => 2 * self.adapt_filter_len,
            Filter::TripleVwma => 3 * self.adapt_filter_len,
            Filter::Llama => self.adapt_filter_len + 1,
            _ => self.adapt_filter_len,
        };
        (self.wvad_len + filter + self.signal_len + 1).max(2 * self.adx_len + 1).max(self.atr_len + 1)
    }

    pub fn validate(&self) -> Result<(), String> {
        let lens = [self.wvad_len, self.signal_len, self.adx_len, self.atr_len];
        if lens.iter().any(|l| !(1..=500).contains(l)) || !(1..=500).contains(&self.adapt_filter_len) {
            return Err("wvad_len, signal_len, adapt_filter_len, adx_len, atr_len must be 1..500".into());
        }
        if self.adapt_filter == Filter::Hma && self.adapt_filter_len < 2 {
            return Err("HMA needs adapt_filter_len >= 2".into());
        }
        if self.timeframe_minutes == 0 {
            return Err("timeframe_minutes must be > 0".into());
        }
        if self.use_htf && (self.htf_minutes <= self.timeframe_minutes || self.htf_minutes % self.timeframe_minutes != 0) {
            return Err("htf_minutes must be a larger multiple of timeframe_minutes".into());
        }
        if !(self.adx_threshold.is_finite() && self.adx_threshold >= 0.0) {
            return Err("adx_threshold must be >= 0".into());
        }
        if !(self.sl_mult.is_finite() && self.sl_mult > 0.0) {
            return Err("sl_mult must be positive".into());
        }
        if !(self.tp1_r > 0.0 && self.tp1_r < self.tp2_r && self.tp2_r < self.tp3_r) {
            return Err("TP multipliers must satisfy 0 < TP1 < TP2 < TP3".into());
        }
        let (f1, f2) = (self.tp1_close_fraction, self.tp2_close_fraction);
        if !(f1 >= 0.0 && f2 >= 0.0 && f1 + f2 < 1.0) {
            return Err("tp1/tp2 close fractions must be >= 0 and sum to < 1".into());
        }
        if !(self.tick_size.is_finite() && self.tick_size > 0.0) {
            return Err("tick_size must be positive".into());
        }
        if self.tp_through_ticks > 100 || self.min_risk_ticks == 0 {
            return Err("tp_through_ticks must be 0..100 and min_risk_ticks >= 1".into());
        }
        if self.warmup_bars < self.min_warmup() {
            return Err(format!("warmup_bars must be at least {} for these lengths", self.min_warmup()));
        }
        Ok(())
    }

    /// Short description for titles and reports, e.g. "WVAD 20 · Sma 9 · signal 9 · SL 1.5 ATR14".
    pub fn describe(&self) -> String {
        let filter = match self.adapt_filter {
            Filter::None => "no filter".to_owned(),
            f => format!("{f:?} {}", self.adapt_filter_len),
        };
        let mut s = format!("WVAD {} · {filter} · signal {} · SL {} ATR{}", self.wvad_len, self.signal_len, self.sl_mult, self.atr_len);
        if self.use_adx {
            s.push_str(&format!(" · ADX{} ≥ {}", self.adx_len, self.adx_threshold));
        }
        if self.use_htf {
            s.push_str(&format!(" · HTF {}m", self.htf_minutes));
        }
        s
    }
}
