//! Machine Learning: Lorentzian Classification (v2.0) — Rust port of the signal and trade model.
//!
//! Source: TradingView Pine v6 indicator "Machine Learning: Lorentzian Classification",
//! © jdehorty, with its libraries `jdehorty/MLExtensions/2` and `jdehorty/KernelFunctions/2`,
//! all Mozilla Public License 2.0. This crate is a derivative work under the same licence.
//!
//! Model (see `doc/LORENTZIAN_STRATEGY.md`):
//! * five normalised features per bar (default RSI 14, WaveTrend 10/11, CCI 20, ADX 20, RSI 9);
//! * training label per bar: close 4 bars ago vs now (as the script: a rise is labelled -1);
//! * approximate nearest neighbours by Lorentzian distance (`Σ ln(1 + |Δf|)`), every 4th bar
//!   skipped, `neighbors_count` kept, prediction = sum of their labels;
//! * signal = sign of the prediction when the volatility / regime / ADX filters pass, else the
//!   previous signal; an entry is a signal change that the Nadaraya-Watson kernel (rational
//!   quadratic) slope and the optional EMA / SMA filters agree with;
//! * exits: the script's fixed 4-bar hold (default) or its dynamic kernel exit, plus an ATR
//!   safety stop the script does not have; an accepted opposite entry reverses.
//!
//! Differences from the script, by design:
//! * neighbour search runs over a rolling window of the last `max_bars_back` bars, oldest
//!   first. The script derives its window from the chart's LAST bar (`last_bar_index`), so its
//!   signals depend on how much history is loaded and, with a long chart, it scans bars
//!   backwards and skips the most recent ones. This port is deterministic and live-safe, but
//!   does not match TradingView bar for bar.
//! * `normalize` (CCI, WaveTrend) uses the running min/max since the model started, as the
//!   script does, so its scale depends on where the history starts.
//! * indicator warm-up (EMA / RMA seeds) as Pine; differences vanish after the warm-up.
//! * colours, labels, the trade-stats table and alert texts are not ported.
pub mod engine;
pub mod params;
pub mod ta;

pub use engine::{Engine, Status, Trade};
pub use params::{ExitMode, Feature, FeatureKind, Params};
pub use wvad::{Bar, Event};

/// The Pine source this crate ports.
pub const SOURCE: &str = "Machine Learning: Lorentzian Classification v2.0 [jdehorty]";
/// Version of this port (the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
