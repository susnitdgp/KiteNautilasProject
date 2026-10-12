//! Slot settings (JSON) shared by every bar-model strategy (WVAD; Sniper parked in backup/):
//! contract, lots and partial exits, costs, entry window and square-off, plus the
//! strategy's own `params`. Used by the backtest and the live runner.
use super::bar_model::BarModel;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, FixedOffset, NaiveTime};
use kite_adapter::http::historical::{Interval, KiteInterval};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// What the shared runner needs from a strategy's `params` block.
pub trait ModelParams: Clone + std::fmt::Debug + Default + PartialEq + Serialize + DeserializeOwned + 'static {
    /// `strategy` value in the config and the portfolio; also the log-event prefix.
    const STRATEGY: &'static str;
    /// Upper-case name for titles, strategy ids and console notes ("WVAD").
    const NAME: &'static str;
    /// Model name for the dashboard and reports ("Williams Variable A/D Pressure").
    const MODEL_TITLE: &'static str;
    /// Version of the ported source, shown in the title.
    const SOURCE_VERSION: &'static str;
    fn timeframe_minutes(&self) -> u32;
    fn validate(&self) -> Result<(), String>;
    /// TP1 / TP2 close fractions as written in the JSON (must be 0: the slot sets them).
    fn fractions(&self) -> (f64, f64);
    fn with_fractions(&self, tp1: f64, tp2: f64) -> Self;
    fn build(&self) -> Box<dyn BarModel>;
    /// Bars of finalised history a live start needs before trading.
    fn warmup_needed(&self) -> usize;
    /// Title suffix, e.g. the resolved preset.
    fn label(&self) -> String;
    /// How the stop behaves after targets, for the dashboard ("step stop").
    fn stop_rule(&self) -> &'static str;
}

/// Live-runner settings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LiveSettings {
    /// Reviewed exchange session calendar JSON (holidays, special sessions).
    pub session_calendar: String,
    /// Broker product; the production client only accepts MIS.
    pub product: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Blackout {
    pub from: NaiveTime,
    pub to: NaiveTime,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, bound(deserialize = "P: ModelParams", serialize = "P: ModelParams"))]
pub struct SlotConfig<P> {
    pub strategy: String,
    pub symbol: String,
    pub instrument_token: u32,
    /// Candle minutes: 3, 5, 10, 15 or 30 (fetched natively from Kite).
    pub bar_minutes: u32,
    pub lots: u32,
    /// Lots closed at TP1 / TP2 (the rest at TP3 / stop). 0 = whole-position model.
    #[serde(default)]
    pub tp1_lots: u32,
    #[serde(default)]
    pub tp2_lots: u32,
    pub point_value: f64,
    /// Per-lot round-trip costs excluding brokerage (taxes, exchange, stamp), in points.
    pub round_trip_cost_points: f64,
    /// Flat brokerage per executed order (Zerodha ₹20).
    #[serde(default)]
    pub brokerage_per_order: f64,
    pub slippage_points_per_side: f64,
    pub live: LiveSettings,
    pub entries_until: NaiveTime,
    pub square_off: NaiveTime,
    /// IST windows [from, to) with no new entries (by signal-bar close), e.g. US data.
    #[serde(default)]
    pub entry_blackouts: Vec<Blackout>,
    #[serde(default)]
    pub params: P,
}

/// The session rules the live strategy applies, without the strategy-specific params.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub lots: u32,
    pub tp1_lots: u32,
    pub tp2_lots: u32,
    pub bar_minutes: u32,
    pub entries_until: NaiveTime,
    pub square_off: NaiveTime,
    pub entry_blackouts: Vec<Blackout>,
}

fn ist(ts: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(ts, 0).expect("timestamp").with_timezone(&FixedOffset::east_opt(19_800).expect("IST"))
}

/// New entries allowed for a signal bar closing at `close_ts` (epoch s): inside
/// 09:00..entries_until IST and outside every blackout.
fn entries_allowed(close_ts: i64, until: NaiveTime, blackouts: &[Blackout]) -> bool {
    let t = ist(close_ts).time();
    t >= NaiveTime::from_hms_opt(9, 0, 0).expect("time") && t < until && !blackouts.iter().any(|w| t >= w.from && t < w.to)
}

impl Rules {
    pub fn entries_allowed_at(&self, close_ts: i64) -> bool {
        entries_allowed(close_ts, self.entries_until, &self.entry_blackouts)
    }
    /// Square-off due for a bar closing at `close_ts`.
    pub fn square_off_due(&self, close_ts: i64) -> bool {
        ist(close_ts).time() >= self.square_off
    }
}

impl<P: ModelParams> SlotConfig<P> {
    pub fn load(path: &str) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("Cannot read {path}"))?;
        let c: Self = serde_json::from_slice(&raw).with_context(|| format!("Invalid {} config {path}", P::STRATEGY))?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.strategy == P::STRATEGY, "strategy must be \"{}\"", P::STRATEGY);
        ensure!(self.instrument_token > 0, "instrument_token required");
        ensure!([3, 5, 10, 15, 30].contains(&self.bar_minutes), "bar_minutes must be 3, 5, 10, 15 or 30");
        ensure!(self.params.timeframe_minutes() == self.bar_minutes, "params.timeframe_minutes must equal bar_minutes");
        ensure!((1..=100).contains(&self.lots), "lots must be 1..100");
        ensure!(self.tp1_lots + self.tp2_lots < self.lots, "tp1_lots + tp2_lots must leave at least one lot for TP3");
        ensure!(self.params.fractions() == (0.0, 0.0), "set partial exits with tp1_lots / tp2_lots, not params fractions");
        ensure!(self.brokerage_per_order >= 0.0, "brokerage_per_order must be >= 0");
        ensure!(self.point_value > 0.0, "point_value must be positive");
        ensure!(self.round_trip_cost_points >= 0.0 && self.slippage_points_per_side >= 0.0, "costs must be >= 0");
        ensure!(self.entries_until <= self.square_off, "entries_until must not be after square_off");
        ensure!(self.entry_blackouts.iter().all(|w| w.from < w.to), "entry_blackouts need from < to");
        ensure!(self.live.product == "MIS", "live.product must be MIS");
        self.params.validate().map_err(anyhow::Error::msg)
    }
    /// Model params with the partial fractions derived from whole lots.
    pub fn engine_params(&self) -> P {
        self.params.with_fractions(self.tp1_lots as f64 / self.lots as f64, self.tp2_lots as f64 / self.lots as f64)
    }
    pub fn model(&self) -> Box<dyn BarModel> {
        self.engine_params().build()
    }
    pub fn rules(&self) -> Rules {
        Rules {
            lots: self.lots,
            tp1_lots: self.tp1_lots,
            tp2_lots: self.tp2_lots,
            bar_minutes: self.bar_minutes,
            entries_until: self.entries_until,
            square_off: self.square_off,
            entry_blackouts: self.entry_blackouts.clone(),
        }
    }
    pub fn interval(&self) -> Interval {
        use KiteInterval as K;
        match self.bar_minutes {
            3 => K::ThreeMinute.native(),
            10 => K::TenMinute.native(),
            15 => K::FifteenMinute.native(),
            30 => K::ThirtyMinute.native(),
            _ => K::FiveMinute.native(),
        }
    }
    pub fn entries_allowed_at(&self, close_ts: i64) -> bool {
        entries_allowed(close_ts, self.entries_until, &self.entry_blackouts)
    }
    /// Square-off due for a bar closing at `close_ts`.
    pub fn square_off_due(&self, close_ts: i64) -> bool {
        ist(close_ts).time() >= self.square_off
    }
}
