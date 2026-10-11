//! ATR and ADX on `nautilus-indicators` 0.64 (streaming, `None` until initialised).
//! Taken from the Precision Sniper crate (parked in `backup/`) so WVAD stands alone.
//!
//! Differences from Pine, limited to the warm-up: Wilder averages seed with the first value
//! (Pine seeds with an SMA of the first `length` values); the gap decays by (1 - 1/len) per
//! bar and is negligible after the 200-bar warm-up. ADX is built from
//! `DirectionalMovement` (+DM/-DM, Wilder), `AverageTrueRange` (Wilder) and a
//! `WilderMovingAverage`, with Pine's `fixnan`.
use nautilus_indicators::{
    average::{MovingAverageType, rma::WilderMovingAverage},
    indicator::{Indicator, MovingAverage},
    momentum::dm::DirectionalMovement,
    volatility::atr::AverageTrueRange,
};

/// `ta.atr(len)`: Wilder average of the true range (first bar: high - low).
#[derive(Debug)]
pub struct Atr(AverageTrueRange);
impl Atr {
    pub fn new(len: usize) -> Self {
        Self(AverageTrueRange::new(len, Some(MovingAverageType::Wilder), Some(true), None))
    }
    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        self.0.update_raw(high, low, close);
        self.0.initialized().then_some(self.0.value)
    }
}

/// `ta.dmi(diLen, adxLen)` → (+DI, -DI, ADX).
#[derive(Debug)]
pub struct Dmi {
    dm: DirectionalMovement,
    tr: AverageTrueRange,
    adx: WilderMovingAverage,
    last_plus: Option<f64>,
    last_minus: Option<f64>,
}
impl Dmi {
    pub fn new(di_len: usize, adx_len: usize) -> Self {
        Self {
            dm: DirectionalMovement::new(di_len, Some(MovingAverageType::Wilder)),
            tr: AverageTrueRange::new(di_len, Some(MovingAverageType::Wilder), Some(true), None),
            adx: WilderMovingAverage::new(adx_len, None),
            last_plus: None,
            last_minus: None,
        }
    }
    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<(f64, f64, f64)> {
        self.dm.update_raw(high, low);
        self.tr.update_raw(high, low, close);
        if !(self.dm.initialized() && self.tr.initialized()) {
            return None;
        }
        // fixnan: keep the last valid value when the true-range average is 0
        let t = self.tr.value;
        if t != 0.0 {
            self.last_plus = Some(100.0 * self.dm.pos / t);
            self.last_minus = Some(100.0 * self.dm.neg / t);
        }
        let (plus, minus) = (self.last_plus?, self.last_minus?);
        let sum = plus + minus;
        self.adx.update_raw((plus - minus).abs() / if sum == 0.0 { 1.0 } else { sum });
        self.adx.initialized().then(|| (plus, minus, 100.0 * self.adx.value()))
    }
}
