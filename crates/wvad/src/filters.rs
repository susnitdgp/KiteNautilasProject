//! Pine `ta.*` building blocks used by WVAD, streaming and Pine-exact; `None` is Pine `na`.
//!
//! The WVAD sum is `na` for its first `wvad_len - 1` bars. Pine propagates a leading `na`
//! through `sma` / `ema` / `rma` / `wma` / `vwma` / `hma` by returning `na` until the
//! window holds only real values, which is the same as starting these filters later:
//! callers simply skip `None` inputs. The Kalman filter is the exception (its error term
//! advances on every bar), so it takes `Option` input.
use crate::params::Filter;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
struct Window {
    len: usize,
    buf: VecDeque<f64>,
}
impl Window {
    fn new(len: usize) -> Self {
        let len = len.max(1);
        Self { len, buf: VecDeque::with_capacity(len + 1) }
    }
    /// Adds `x`; true once the window is full.
    fn push(&mut self, x: f64) -> bool {
        self.buf.push_back(x);
        if self.buf.len() > self.len {
            self.buf.pop_front();
        }
        self.buf.len() == self.len
    }
}

/// `ta.sma(src, len)`. The sum is recomputed each bar (no running-sum drift).
#[derive(Debug, Clone)]
pub struct Sma(Window);
impl Sma {
    pub fn new(len: usize) -> Self {
        Self(Window::new(len))
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        if !self.0.push(x) {
            return None;
        }
        Some(self.0.buf.iter().sum::<f64>() / self.0.len as f64)
    }
}

/// `ta.ema(src, len)`: alpha 2 / (len + 1), seeded with the first value.
#[derive(Debug, Clone)]
pub struct Ema {
    alpha: f64,
    value: Option<f64>,
}
impl Ema {
    pub fn new(len: usize) -> Self {
        Self { alpha: 2.0 / (len.max(1) as f64 + 1.0), value: None }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        let v = match self.value {
            None => x,
            Some(p) => self.alpha * x + (1.0 - self.alpha) * p,
        };
        self.value = Some(v);
        self.value
    }
}

/// `ta.rma(src, len)`: alpha 1 / len, seeded with the SMA of the first `len` values.
#[derive(Debug, Clone)]
pub struct Rma {
    alpha: f64,
    seed: Sma,
    value: Option<f64>,
}
impl Rma {
    pub fn new(len: usize) -> Self {
        Self { alpha: 1.0 / len.max(1) as f64, seed: Sma::new(len), value: None }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        let v = match self.value {
            Some(p) => self.alpha * x + (1.0 - self.alpha) * p,
            None => self.seed.update(x)?,
        };
        self.value = Some(v);
        self.value
    }
}

/// `ta.wma(src, len)`: weights 1..len, the newest value weighs `len`.
#[derive(Debug, Clone)]
pub struct Wma(Window);
impl Wma {
    pub fn new(len: usize) -> Self {
        Self(Window::new(len))
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        if !self.0.push(x) {
            return None;
        }
        let n = self.0.len as f64;
        let sum: f64 = self.0.buf.iter().enumerate().map(|(i, v)| v * (i + 1) as f64).sum();
        Some(sum / (n * (n + 1.0) / 2.0))
    }
}

/// `ta.vwma(src, len)` = sma(src × volume) / sma(volume); `na` when the volume average is 0.
#[derive(Debug, Clone)]
pub struct Vwma {
    weighted: Sma,
    volume: Sma,
}
impl Vwma {
    pub fn new(len: usize) -> Self {
        Self { weighted: Sma::new(len), volume: Sma::new(len) }
    }
    pub fn update(&mut self, x: f64, volume: f64) -> Option<f64> {
        let a = self.weighted.update(x * volume);
        let b = self.volume.update(volume);
        match (a, b) {
            (Some(a), Some(b)) if b != 0.0 => Some(a / b),
            _ => None,
        }
    }
}

/// `ta.hma(src, len)` = wma(2 × wma(src, floor(len / 2)) - wma(src, len), round(sqrt(len))).
#[derive(Debug, Clone)]
pub struct Hma {
    half: Wma,
    full: Wma,
    out: Wma,
}
impl Hma {
    pub fn new(len: usize) -> Self {
        let len = len.max(2);
        Self { half: Wma::new(len / 2), full: Wma::new(len), out: Wma::new(((len as f64).sqrt().round() as usize).max(1)) }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        let h = self.half.update(x);
        let f = self.full.update(x);
        match (h, f) {
            (Some(h), Some(f)) => self.out.update(2.0 * h - f),
            _ => None,
        }
    }
}

/// The script's LLAMA: sma(src, len) + (src - src[len]) / len × (len / 2).
#[derive(Debug, Clone)]
pub struct Llama {
    len: usize,
    mean: Sma,
    history: VecDeque<f64>,
}
impl Llama {
    pub fn new(len: usize) -> Self {
        let len = len.max(1);
        Self { len, mean: Sma::new(len), history: VecDeque::with_capacity(len + 2) }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        let mean = self.mean.update(x);
        self.history.push_back(x);
        if self.history.len() > self.len + 1 {
            self.history.pop_front();
        }
        let mean = mean?;
        if self.history.len() < self.len + 1 {
            return None;
        }
        let n = self.len as f64;
        Some(mean + (x - self.history[0]) / n * (n / 2.0))
    }
}

/// The script's one-dimensional Kalman filter: q = 1 / len, r = 1, error starts at 1.
/// The error term advances on every bar, even while the source is `na`.
#[derive(Debug, Clone)]
pub struct Kalman {
    q: f64,
    est: Option<f64>,
    err: f64,
}
impl Kalman {
    pub fn new(len: usize) -> Self {
        Self { q: 1.0 / len.max(1) as f64, est: None, err: 1.0 }
    }
    pub fn update(&mut self, x: Option<f64>) -> Option<f64> {
        let pred = self.err + self.q;
        let gain = pred / (pred + 1.0);
        let est = self.est.or(x);
        self.est = match (est, x) {
            (Some(e), Some(s)) => Some(e + gain * (s - e)),
            _ => None,
        };
        self.err = (1.0 - gain) * pred;
        self.est
    }
}

/// Highest value of the last `len` inputs (`ta.highest`).
#[derive(Debug, Clone)]
pub struct Highest(Window);
impl Highest {
    pub fn new(len: usize) -> Self {
        Self(Window::new(len))
    }
    pub fn update(&mut self, x: f64) -> f64 {
        self.0.push(x);
        self.0.buf.iter().copied().fold(f64::MIN, f64::max)
    }
}

/// The selected adaptive filter.
#[derive(Debug, Clone)]
pub enum Adaptive {
    None,
    Sma(Sma),
    Ema(Ema),
    Rma(Rma),
    DoubleWma(Wma, Wma),
    TripleVwma(Vwma, Vwma, Vwma),
    Hma(Hma),
    Llama(Llama),
    Kalman(Kalman),
}

impl Adaptive {
    pub fn new(filter: Filter, len: usize) -> Self {
        match filter {
            Filter::None => Self::None,
            Filter::Sma => Self::Sma(Sma::new(len)),
            Filter::Ema => Self::Ema(Ema::new(len)),
            Filter::Rma => Self::Rma(Rma::new(len)),
            Filter::DoubleWma => Self::DoubleWma(Wma::new(len), Wma::new(len)),
            Filter::TripleVwma => Self::TripleVwma(Vwma::new(len), Vwma::new(len), Vwma::new(len)),
            Filter::Hma => Self::Hma(Hma::new(len)),
            Filter::Llama => Self::Llama(Llama::new(len)),
            Filter::Kalman => Self::Kalman(Kalman::new(len)),
        }
    }

    /// One bar: `x` is the WVAD sum (`None` while it is `na`), `volume` the bar volume.
    pub fn update(&mut self, x: Option<f64>, volume: f64) -> Option<f64> {
        if let Self::Kalman(k) = self {
            return k.update(x);
        }
        let x = x?;
        match self {
            Self::None => Some(x),
            Self::Sma(f) => f.update(x),
            Self::Ema(f) => f.update(x),
            Self::Rma(f) => f.update(x),
            Self::DoubleWma(a, b) => {
                let v = a.update(x)?;
                b.update(v)
            }
            Self::TripleVwma(a, b, c) => {
                let v1 = a.update(x, volume)?;
                let v2 = b.update(v1, volume)?;
                c.update(v2, volume)
            }
            Self::Hma(f) => f.update(x),
            Self::Llama(f) => f.update(x),
            Self::Kalman(_) => unreachable!("handled above"),
        }
    }
}
