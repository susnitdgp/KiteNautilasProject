//! Williams Variable A/D Pressure [MarkitTick] — Rust port of the signal and trade model.
//!
//! Source: TradingView Pine v6 indicator "Williams Variable A/D Pressure [MarkitTick]",
//! © MarkitTick, licensed CC BY-NC-SA 4.0 (https://creativecommons.org/licenses/by-nc-sa/4.0/).
//! This crate is a derivative work under the same licence.
//!
//! Model (see `doc/WVAD_STRATEGY.md`):
//! * raw WVAD per bar = (close - open) / (high - low) × volume (0 when high = low);
//! * WVAD = its rolling sum over `wvad_len` bars, smoothed by the adaptive filter
//!   (SMA by default), signal line = SMA of that over `signal_len`;
//! * long when WVAD crosses above the signal line, short when it crosses below, on a
//!   confirmed bar; optional ADX and higher-timeframe candle filters;
//! * entry at the crossover bar's close, stop = entry ∓ ATR × `sl_mult`, targets at
//!   1R / 2R / 3R; SL or TP3 closes the trade, an accepted opposite signal reverses it.
//!
//! The script detects the cross one bar late (`crossover(wvad[1], signal[1])`) and draws
//! the levels from `close[1]` / `atr[1]`, i.e. from the crossover bar itself. This port acts
//! at that bar's close, so its levels are the script's levels exactly and the order goes
//! out one bar earlier than the script's alert. Dashboard, colours, lines and alert
//! payloads are not ported.
pub mod engine;
pub mod filters;
pub mod params;

pub use engine::{Engine, Status, Trade};
pub use params::Params;
pub use sniper::{Bar, Event};

/// The Pine source this crate ports (it has no version string of its own).
pub const SOURCE: &str = "Williams Variable A/D Pressure [MarkitTick]";
/// Version of this port (the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
