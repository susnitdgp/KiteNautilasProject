//! Inputs of Machine Learning: Lorentzian Classification that affect the trade model, plus the
//! exit and stop settings a strategy needs (the script is an indicator). Display, colour,
//! trade-stats and alert inputs are not ported; "Include Full History" is replaced by the
//! rolling `max_bars_back` window (see lib.rs).
use serde::{Deserialize, Serialize};

/// Feature type ("Feature 1..5").
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FeatureKind {
    #[serde(rename = "RSI")]
    Rsi,
    #[serde(rename = "WT")]
    Wt,
    #[serde(rename = "CCI")]
    Cci,
    #[serde(rename = "ADX")]
    Adx,
}

/// One feature: type, "Parameter A" and "Parameter B". RSI and CCI: length A, smoothed by an
/// EMA of length B; WT: lengths A and B; ADX: length A (B unused).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub kind: FeatureKind,
    pub a: usize,
    pub b: usize,
}

impl Feature {
    pub const fn new(kind: FeatureKind, a: usize, b: usize) -> Self {
        Self { kind, a, b }
    }
    fn short(&self) -> String {
        match self.kind {
            FeatureKind::Rsi => format!("RSI{}", self.a),
            FeatureKind::Wt => format!("WT{}/{}", self.a, self.b),
            FeatureKind::Cci => format!("CCI{}", self.a),
            FeatureKind::Adx => format!("ADX{}", self.a),
        }
    }
}

/// How an open trade ends (besides the stop, a reversal and the runner's square-off).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ExitMode {
    /// The script's default: close at the 4th bar after entry (the label horizon).
    Fixed,
    /// The script's "Use Dynamic Exits": close when the kernel slope turns against the trade.
    Dynamic,
}

/// Bars a trade is held in `Fixed` mode; hard-coded in the script (training label horizon).
pub const HOLD_BARS: u64 = 4;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Chart timeframe, minutes (must equal the slot's `bar_minutes`).
    pub timeframe_minutes: u32,
    /// "Neighbors Count".
    pub neighbors_count: usize,
    /// "Max Bars Back": size of the rolling neighbour window (bars, including the current one).
    pub max_bars_back: usize,
    /// "Feature Count": how many of `features` enter the distance (2..5).
    pub feature_count: usize,
    pub features: [Feature; 5],
    /// "Use Volatility Filter": ATR(1) > ATR(10).
    pub use_volatility_filter: bool,
    /// "Use Regime Filter" and its threshold (normalised KLMF slope decline).
    pub use_regime_filter: bool,
    pub regime_threshold: f64,
    /// "Use ADX Filter": ADX(14) > threshold.
    pub use_adx_filter: bool,
    pub adx_threshold: f64,
    /// "Use EMA Filter" / "Use SMA Filter" on close.
    pub use_ema_filter: bool,
    pub ema_period: usize,
    pub use_sma_filter: bool,
    pub sma_period: usize,
    /// "Trade with Kernel": entries need the kernel slope (or crossover) in their direction.
    pub use_kernel_filter: bool,
    /// "Enhance Kernel Smoothing": Gaussian/rational-quadratic crossover instead of slope.
    pub use_kernel_smoothing: bool,
    /// Kernel "Lookback Window" (h), "Relative Weighting" (r), "Regression Level" (x), "Lag".
    pub kernel_lookback: usize,
    pub kernel_relative_weight: f64,
    pub kernel_regression_level: usize,
    pub kernel_lag: usize,
    /// Fixed = the script's 4-bar exit (default), Dynamic = "Use Dynamic Exits".
    pub exit_mode: ExitMode,
    /// Not in the script. Fixed mode only: close early when the model signal flips against
    /// the trade before the 4th bar (the script's trade stats count that as a stop-loss).
    pub exit_on_signal_flip: bool,
    /// Not in the script: protective stop = entry ∓ ATR(`atr_len`) × `sl_mult`, stop-first on
    /// bar OHLC. Also the 1R unit of the report.
    pub atr_len: usize,
    pub sl_mult: f64,
    pub tick_size: f64,
    /// Trades whose stop distance is below this many ticks are skipped.
    pub min_risk_ticks: u32,
    /// Bars before the first signal may be taken (window filled, indicators settled).
    pub warmup_bars: usize,
    /// Backtest only: calendar days of history fetched before FROM to warm the model.
    pub history_days: i64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            timeframe_minutes: 5,
            neighbors_count: 8,
            max_bars_back: 2000,
            feature_count: 5,
            features: [
                Feature::new(FeatureKind::Rsi, 14, 1),
                Feature::new(FeatureKind::Wt, 10, 11),
                Feature::new(FeatureKind::Cci, 20, 1),
                Feature::new(FeatureKind::Adx, 20, 2),
                Feature::new(FeatureKind::Rsi, 9, 1),
            ],
            use_volatility_filter: true,
            use_regime_filter: true,
            regime_threshold: -0.1,
            use_adx_filter: false,
            adx_threshold: 20.0,
            use_ema_filter: false,
            ema_period: 200,
            use_sma_filter: false,
            sma_period: 200,
            use_kernel_filter: true,
            use_kernel_smoothing: false,
            kernel_lookback: 8,
            kernel_relative_weight: 8.0,
            kernel_regression_level: 25,
            kernel_lag: 2,
            exit_mode: ExitMode::Fixed,
            exit_on_signal_flip: true,
            atr_len: 14,
            sl_mult: 2.0,
            tick_size: 1.0,
            min_risk_ticks: 1,
            warmup_bars: 2000,
            history_days: 21,
        }
    }
}

impl Params {
    /// Fewest bars after which every indicator the model uses has a value.
    pub fn min_warmup(&self) -> usize {
        let feats = self.features.iter().map(|f| f.a + f.b + 4).max().unwrap_or(0);
        let mut m = feats.max(self.kernel_regression_level + 3).max(self.atr_len + 1);
        if self.use_regime_filter {
            m = m.max(201);
        }
        if self.use_ema_filter {
            m = m.max(self.ema_period + 1);
        }
        if self.use_sma_filter {
            m = m.max(self.sma_period + 1);
        }
        m
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.timeframe_minutes == 0 {
            return Err("timeframe_minutes must be > 0".into());
        }
        if !(1..=100).contains(&self.neighbors_count) {
            return Err("neighbors_count must be 1..100".into());
        }
        if !(100..=20_000).contains(&self.max_bars_back) {
            return Err("max_bars_back must be 100..20000".into());
        }
        if !(2..=5).contains(&self.feature_count) {
            return Err("feature_count must be 2..5".into());
        }
        if self.features.iter().any(|f| !(1..=500).contains(&f.a) || !(1..=500).contains(&f.b)) {
            return Err("feature parameters A and B must be 1..500".into());
        }
        if !(self.regime_threshold.is_finite() && (-10.0..=10.0).contains(&self.regime_threshold)) {
            return Err("regime_threshold must be -10..10".into());
        }
        if !(self.adx_threshold.is_finite() && (0.0..=100.0).contains(&self.adx_threshold)) {
            return Err("adx_threshold must be 0..100".into());
        }
        if self.ema_period == 0 || self.sma_period == 0 || self.ema_period > 1000 || self.sma_period > 1000 {
            return Err("ema_period and sma_period must be 1..1000".into());
        }
        if self.kernel_lookback < 3 || self.kernel_lookback <= self.kernel_lag {
            return Err("kernel_lookback must be >= 3 and greater than kernel_lag".into());
        }
        if !(self.kernel_relative_weight.is_finite() && self.kernel_relative_weight > 0.0) {
            return Err("kernel_relative_weight must be positive".into());
        }
        if !(1..=200).contains(&self.kernel_regression_level) {
            return Err("kernel_regression_level must be 1..200".into());
        }
        if self.exit_mode == ExitMode::Dynamic && (self.use_ema_filter || self.use_sma_filter || self.use_kernel_smoothing) {
            // the script silently falls back to the fixed exit in this case
            return Err("Dynamic exits need the EMA/SMA filters and kernel smoothing off (as the script)".into());
        }
        if !(1..=500).contains(&self.atr_len) {
            return Err("atr_len must be 1..500".into());
        }
        if !(self.sl_mult.is_finite() && self.sl_mult > 0.0) {
            return Err("sl_mult must be positive".into());
        }
        if !(self.tick_size.is_finite() && self.tick_size > 0.0) || self.min_risk_ticks == 0 {
            return Err("tick_size must be positive and min_risk_ticks >= 1".into());
        }
        if self.warmup_bars < self.min_warmup() {
            return Err(format!("warmup_bars must be at least {} for these settings", self.min_warmup()));
        }
        if !(1..=90).contains(&self.history_days) {
            return Err("history_days must be 1..90".into());
        }
        Ok(())
    }

    /// Short description for titles and reports.
    pub fn describe(&self) -> String {
        let feats: Vec<String> = self.features[..self.feature_count].iter().map(Feature::short).collect();
        let mut s = format!("LDC k{} · window {} · {}", self.neighbors_count, self.max_bars_back, feats.join(" "));
        let mut filters = Vec::new();
        if self.use_volatility_filter {
            filters.push("vol".to_owned());
        }
        if self.use_regime_filter {
            filters.push(format!("regime {}", self.regime_threshold));
        }
        if self.use_adx_filter {
            filters.push(format!("ADX>{}", self.adx_threshold));
        }
        if self.use_ema_filter {
            filters.push(format!("EMA{}", self.ema_period));
        }
        if self.use_sma_filter {
            filters.push(format!("SMA{}", self.sma_period));
        }
        if !filters.is_empty() {
            s.push_str(&format!(" · filters {}", filters.join("+")));
        }
        if self.use_kernel_filter {
            s.push_str(&format!(
                " · kernel {}/{}/{}{}",
                self.kernel_lookback,
                self.kernel_relative_weight,
                self.kernel_regression_level,
                if self.use_kernel_smoothing { " smooth" } else { "" }
            ));
        }
        match self.exit_mode {
            ExitMode::Fixed => s.push_str(&format!(" · exit {HOLD_BARS} bars{}", if self.exit_on_signal_flip { " or flip" } else { "" })),
            ExitMode::Dynamic => s.push_str(" · dynamic kernel exit"),
        }
        s.push_str(&format!(" · SL {} ATR{}", self.sl_mult, self.atr_len));
        s
    }
}
