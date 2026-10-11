//! The interface between a strategy's bar-by-bar trade model (crate `wvad`; Precision Sniper
//! is parked in `backup/`) and the shared runner (`model_strategy`, `model_backtest`,
//! `model_live`). Every model speaks `wvad::Bar` in and `wvad::Event` out:
//! Entry → Partial (TP1 / TP2) → Exit.
use wvad::{Bar, Event};
use std::collections::BTreeMap;

/// The open model trade, as the runner needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub dir: i32,
    pub entry: f64,
    pub stop: f64,
    pub initial_stop: f64,
    pub tps: [f64; 3],
    pub hits: [bool; 3],
    /// Open fraction of the original position.
    pub remaining: f64,
}

pub trait BarModel: std::fmt::Debug {
    /// One confirmed bar; `entries_allowed` = false: no new trades (open ones are managed).
    fn on_bar(&mut self, b: Bar, entries_allowed: bool) -> Vec<Event>;
    /// Closes the open model trade at `price` (live stop on a tick, square-off).
    fn force_close(&mut self, price: f64, reason: &'static str) -> Option<Event>;
    /// Live: a tick reached target `level` (1..=3) before the bar closed.
    fn mark_target(&mut self, level: u8) -> Vec<Event>;
    fn levels(&self) -> Option<Levels>;
    /// Indicators warm and signals possible.
    fn ready(&self) -> bool;
    /// 1 bullish, -1 bearish, 0 neutral (dashboard trend, warm-up message).
    fn trend(&self) -> i32;
    /// Dashboard "model" rows.
    fn model_rows(&self) -> Vec<(String, String)>;
    /// Strategy-specific fields of the per-bar log line.
    fn bar_log(&self) -> serde_json::Value;
    /// Resolved settings for the backtest report.
    fn describe(&self) -> String;
    /// Signal candidates seen and why the rejected ones were not taken.
    fn candidates(&self) -> u32;
    fn rejected(&self) -> BTreeMap<String, u32>;
}
