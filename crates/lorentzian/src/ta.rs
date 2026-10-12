//! Streaming Pine built-ins and the MLExtensions v2 / KernelFunctions v2 functions the script
//! uses, bar for bar. Pine `na` is `f64::NAN`; built-in averages skip `na` inputs.
//!
//! Written here rather than on `nautilus-indicators`: the features are Pine-specific
//! (normalised with running min/max, jdehorty's own ADX with `nz(x[1]) = 0` on the first bar,
//! CCI on close, kernels), and Pine's RSI / EMA seeding differs from the library defaults.
use std::collections::VecDeque;

/// Pine `nz`: `na` → 0.
#[inline]
pub fn nz(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v }
}

/// Pine float division: `x / 0` is `na`.
#[inline]
fn div(a: f64, b: f64) -> f64 {
    if b == 0.0 { f64::NAN } else { a / b }
}

/// `ta.sma(src, len)`; `na` inputs skipped.
#[derive(Debug, Clone)]
pub struct Sma {
    len: usize,
    buf: VecDeque<f64>,
}
impl Sma {
    pub fn new(len: usize) -> Self {
        Self { len: len.max(1), buf: VecDeque::with_capacity(len.max(1) + 1) }
    }
    pub fn update(&mut self, v: f64) -> f64 {
        if !v.is_nan() {
            self.buf.push_back(v);
            if self.buf.len() > self.len {
                self.buf.pop_front();
            }
        }
        if self.buf.len() == self.len { self.buf.iter().sum::<f64>() / self.len as f64 } else { f64::NAN }
    }
}

/// Exponential average with Pine's seeding: `na` until `len` values, then seeded with their
/// SMA. `alpha` = 2/(len+1) for `ta.ema`, 1/len for `ta.rma`. `na` inputs keep the last value.
#[derive(Debug, Clone)]
pub struct ExpAvg {
    len: usize,
    alpha: f64,
    seed: Vec<f64>,
    value: f64,
}
impl ExpAvg {
    pub fn ema(len: usize) -> Self {
        let len = len.max(1);
        Self { len, alpha: 2.0 / (len as f64 + 1.0), seed: Vec::with_capacity(len), value: f64::NAN }
    }
    pub fn rma(len: usize) -> Self {
        let len = len.max(1);
        Self { len, alpha: 1.0 / len as f64, seed: Vec::with_capacity(len), value: f64::NAN }
    }
    pub fn update(&mut self, v: f64) -> f64 {
        if v.is_nan() {
            return self.value;
        }
        if self.value.is_nan() {
            self.seed.push(v);
            if self.seed.len() == self.len {
                self.value = self.seed.iter().sum::<f64>() / self.len as f64;
                self.seed = Vec::new();
            }
        } else {
            self.value = self.alpha * v + (1.0 - self.alpha) * self.value;
        }
        self.value
    }
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// `ta.rsi(src, len)` (Wilder averages of gains / losses).
#[derive(Debug, Clone)]
pub struct Rsi {
    prev: Option<f64>,
    up: ExpAvg,
    down: ExpAvg,
}
impl Rsi {
    pub fn new(len: usize) -> Self {
        Self { prev: None, up: ExpAvg::rma(len), down: ExpAvg::rma(len) }
    }
    pub fn update(&mut self, src: f64) -> f64 {
        let Some(p) = self.prev.replace(src) else { return f64::NAN };
        let ch = src - p;
        let u = self.up.update(ch.max(0.0));
        let d = self.down.update((-ch).max(0.0));
        if u.is_nan() || d.is_nan() {
            f64::NAN
        } else if d == 0.0 {
            100.0
        } else if u == 0.0 {
            0.0
        } else {
            100.0 - 100.0 / (1.0 + u / d)
        }
    }
}

/// `ta.cci(src, len)` = (src - SMA) / (0.015 × mean absolute deviation).
#[derive(Debug, Clone)]
pub struct Cci {
    len: usize,
    buf: VecDeque<f64>,
}
impl Cci {
    pub fn new(len: usize) -> Self {
        Self { len: len.max(1), buf: VecDeque::with_capacity(len.max(1) + 1) }
    }
    pub fn update(&mut self, src: f64) -> f64 {
        self.buf.push_back(src);
        if self.buf.len() > self.len {
            self.buf.pop_front();
        }
        if self.buf.len() < self.len {
            return f64::NAN;
        }
        let n = self.len as f64;
        let mean = self.buf.iter().sum::<f64>() / n;
        let dev = self.buf.iter().map(|x| (x - mean).abs()).sum::<f64>() / n;
        div(src - mean, 0.015 * dev)
    }
}

/// MLExtensions `normalize(src, 0, 1)`: rescale by the running min / max of every value seen.
#[derive(Debug, Clone)]
pub struct Normalize {
    min: f64,
    max: f64,
}
impl Default for Normalize {
    fn default() -> Self {
        Self { min: 10e10, max: -10e10 }
    }
}
impl Normalize {
    pub fn update(&mut self, src: f64) -> f64 {
        if src.is_nan() {
            return f64::NAN;
        }
        self.min = self.min.min(src);
        self.max = self.max.max(src);
        (src - self.min) / (self.max - self.min).max(10e-10)
    }
}

/// MLExtensions `rescale(src, oldMin, oldMax, 0, 1)`.
#[inline]
pub fn rescale(src: f64, old_min: f64, old_max: f64) -> f64 {
    (src - old_min) / (old_max - old_min).max(10e-10)
}

/// MLExtensions `n_wt(src, n1, n2)`: WaveTrend Classic, `normalize(wt1 - wt2)`.
#[derive(Debug, Clone)]
pub struct WaveTrend {
    ema1: ExpAvg,
    ema2: ExpAvg,
    wt1: ExpAvg,
    wt2: Sma,
    norm: Normalize,
}
impl WaveTrend {
    pub fn new(n1: usize, n2: usize) -> Self {
        Self { ema1: ExpAvg::ema(n1), ema2: ExpAvg::ema(n1), wt1: ExpAvg::ema(n2), wt2: Sma::new(4), norm: Normalize::default() }
    }
    pub fn update(&mut self, src: f64) -> f64 {
        let e1 = self.ema1.update(src);
        let e2 = self.ema2.update((src - e1).abs());
        let ci = div(src - e1, 0.015 * e2);
        let w1 = self.wt1.update(ci);
        let w2 = self.wt2.update(w1);
        self.norm.update(w1 - w2)
    }
}

/// jdehorty's directional-movement ADX (MLExtensions `n_adx` / `filter_adx`): Wilder running
/// sums with `nz(x[1])` = 0 on the first bar, ADX = `ta.rma(dx, len)`. Unscaled (0..100).
#[derive(Debug, Clone)]
pub struct JAdx {
    len: f64,
    prev: Option<(f64, f64, f64)>,
    tr_s: f64,
    plus_s: f64,
    minus_s: f64,
    adx: ExpAvg,
}
impl JAdx {
    pub fn new(len: usize) -> Self {
        let len = len.max(1);
        Self { len: len as f64, prev: None, tr_s: 0.0, plus_s: 0.0, minus_s: 0.0, adx: ExpAvg::rma(len) }
    }
    pub fn update(&mut self, h: f64, l: f64, c: f64) -> f64 {
        let (ph, pl, pc) = self.prev.replace((h, l, c)).unwrap_or((0.0, 0.0, 0.0));
        let tr = (h - l).max((h - pc).abs()).max((l - pc).abs());
        let up = h - ph;
        let down = pl - l;
        let plus = if up > down { up.max(0.0) } else { 0.0 };
        let minus = if down > up { down.max(0.0) } else { 0.0 };
        self.tr_s = self.tr_s - self.tr_s / self.len + tr;
        self.plus_s = self.plus_s - self.plus_s / self.len + plus;
        self.minus_s = self.minus_s - self.minus_s / self.len + minus;
        let di_plus = div(self.plus_s, self.tr_s) * 100.0;
        let di_minus = div(self.minus_s, self.tr_s) * 100.0;
        let dx = div((di_plus - di_minus).abs(), di_plus + di_minus) * 100.0;
        self.adx.update(dx)
    }
}

/// `ta.tr(true)`: true range, first bar high - low.
#[derive(Debug, Clone, Default)]
pub struct TrueRange {
    prev_close: Option<f64>,
}
impl TrueRange {
    pub fn update(&mut self, h: f64, l: f64, c: f64) -> f64 {
        let tr = match self.prev_close {
            None => h - l,
            Some(pc) => (h - l).max((h - pc).abs()).max((l - pc).abs()),
        };
        self.prev_close = Some(c);
        tr
    }
}

/// MLExtensions `regime_filter` value: normalised decline of the KLMF curve slope (the filter
/// passes when it is >= the threshold).
#[derive(Debug, Clone)]
pub struct Regime {
    prev_src: Option<f64>,
    value1: f64,
    value2: f64,
    klmf: f64,
    ema: ExpAvg,
}
impl Default for Regime {
    fn default() -> Self {
        Self { prev_src: None, value1: f64::NAN, value2: f64::NAN, klmf: f64::NAN, ema: ExpAvg::ema(200) }
    }
}
impl Regime {
    pub fn update(&mut self, src: f64, h: f64, l: f64) -> f64 {
        let v1 = match self.prev_src.replace(src) {
            None => f64::NAN,
            Some(p) => 0.2 * (src - p) + 0.8 * nz(self.value1),
        };
        let v2 = 0.1 * (h - l) + 0.8 * nz(self.value2);
        self.value1 = v1;
        self.value2 = v2;
        let omega = div(v1, v2).abs();
        let o2 = omega * omega;
        let alpha = (-o2 + (o2 * o2 + 16.0 * o2).sqrt()) / 8.0;
        let prev = self.klmf;
        self.klmf = alpha * src + (1.0 - alpha) * nz(prev);
        let slope = (self.klmf - prev).abs();
        let avg = self.ema.update(slope);
        div(slope - avg, avg)
    }
}

/// KernelFunctions v2 `rationalQuadratic(src, h, r, x)` over `closes` (newest last); the
/// script sums bars 0..=x+1. `None` until enough history.
pub fn rational_quadratic(closes: &VecDeque<f64>, h: usize, r: f64, x: usize) -> Option<f64> {
    kernel(closes, x, |i| (1.0 + (i * i) / ((h as f64).powi(2) * 2.0 * r)).powf(-r))
}

/// KernelFunctions v2 `gaussian(src, h, x)`.
pub fn gaussian(closes: &VecDeque<f64>, h: usize, x: usize) -> Option<f64> {
    kernel(closes, x, |i| (-(i * i) / (2.0 * (h as f64).powi(2))).exp())
}

fn kernel(closes: &VecDeque<f64>, x: usize, weight: impl Fn(f64) -> f64) -> Option<f64> {
    let n = closes.len();
    if n < x + 2 {
        return None;
    }
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..=x + 1 {
        let w = weight(i as f64);
        num += closes[n - 1 - i] * w;
        den += w;
    }
    Some(num / den)
}
