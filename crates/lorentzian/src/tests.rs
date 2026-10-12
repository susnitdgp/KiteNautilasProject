use crate::{
    Bar, Engine, Event, ExitMode, Params,
    ta::{ExpAvg, Normalize, Rsi, gaussian, rational_quadratic},
};
use std::collections::VecDeque;

/// Deterministic choppy market: two sine waves plus a pseudo-random walk, 5m bars.
fn bars(n: usize) -> Vec<Bar> {
    let mut out = Vec::with_capacity(n);
    let mut px = 8000.0_f64;
    let mut seed = 12345_u64;
    for k in 0..n {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let noise = ((seed >> 33) % 21) as f64 - 10.0;
        let drift = (k as f64 / 17.0).sin() * 6.0 + (k as f64 / 61.0).cos() * 4.0;
        let o = px;
        let c = (px + drift + noise * 0.5).round();
        let h = o.max(c) + 2.0 + ((seed >> 20) % 4) as f64;
        let l = o.min(c) - 2.0 - ((seed >> 24) % 4) as f64;
        out.push(Bar { start: 1_790_000_000 + k as i64 * 300, open: o, high: h, low: l, close: c, volume: 100.0 });
        px = c;
    }
    out
}

fn small() -> Params {
    Params { max_bars_back: 400, warmup_bars: 400, ..Params::default() }
}

#[test]
fn defaults_are_the_script_defaults_and_valid() {
    let p = Params::default();
    p.validate().unwrap();
    assert_eq!((p.neighbors_count, p.max_bars_back, p.feature_count), (8, 2000, 5));
    assert_eq!((p.kernel_lookback, p.kernel_relative_weight, p.kernel_regression_level, p.kernel_lag), (8, 8.0, 25, 2));
    assert!(p.use_volatility_filter && p.use_regime_filter && !p.use_adx_filter && p.use_kernel_filter);
    assert_eq!(p.regime_threshold, -0.1);
    assert_eq!(p.exit_mode, ExitMode::Fixed);
    let json = serde_json::to_string(&p).unwrap();
    let back: Params = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}

#[test]
fn dynamic_exit_with_smoothing_is_rejected() {
    let p = Params { exit_mode: ExitMode::Dynamic, use_kernel_smoothing: true, ..Params::default() };
    assert!(p.validate().is_err());
}

#[test]
fn kernels_reproduce_a_flat_series_and_need_x_plus_2_bars() {
    let flat: VecDeque<f64> = std::iter::repeat_n(100.0, 27).collect();
    assert!((rational_quadratic(&flat, 8, 8.0, 25).unwrap() - 100.0).abs() < 1e-9);
    assert!((gaussian(&flat, 6, 25).unwrap() - 100.0).abs() < 1e-9);
    let short: VecDeque<f64> = std::iter::repeat_n(100.0, 26).collect();
    assert!(rational_quadratic(&short, 8, 8.0, 25).is_none());
    // the newest bar weighs most: a rising series ends above its plain mean
    let rising: VecDeque<f64> = (0..27).map(f64::from).collect();
    assert!(rational_quadratic(&rising, 8, 8.0, 25).unwrap() > 13.0);
}

#[test]
fn pine_seeding_and_normalize() {
    let mut e = ExpAvg::ema(3);
    assert!(e.update(1.0).is_nan());
    assert!(e.update(2.0).is_nan());
    assert_eq!(e.update(3.0), 2.0); // SMA seed
    assert_eq!(e.update(f64::NAN), 2.0); // na skipped
    assert_eq!(e.update(4.0), 3.0);
    let mut r = Rsi::new(2);
    assert!(r.update(10.0).is_nan());
    r.update(11.0);
    assert_eq!(r.update(12.0), 100.0); // no losses
    let mut n = Normalize::default();
    assert_eq!(n.update(5.0), 0.0);
    assert_eq!(n.update(15.0), 1.0);
    assert_eq!(n.update(10.0), 0.5);
}

#[test]
fn rising_close_is_labelled_short_like_the_script() {
    // a steady uptrend: every label is -1, so the neighbours vote short
    let mut e = Engine::new(Params { max_bars_back: 200, warmup_bars: 210, ..Params::default() });
    for k in 0..300 {
        let c = 8000.0 + k as f64;
        e.on_bar(Bar { start: k, open: c - 1.0, high: c + 1.0, low: c - 2.0, close: c, volume: 1.0 }, true);
    }
    assert!(e.status().prediction < 0.0, "prediction {}", e.status().prediction);
}

#[test]
fn trades_follow_the_exit_rules() {
    let p = small();
    let mut e = Engine::new(p.clone());
    let data = bars(3000);
    let mut entries = 0;
    let mut open: Option<(usize, i32)> = None;
    for (i, b) in data.iter().enumerate() {
        for ev in e.on_bar(*b, true) {
            match ev {
                Event::Entry { dir, stop, price, tp1, score, score_max, .. } => {
                    assert!(open.is_none(), "entry while open");
                    assert!(tp1.is_nan());
                    assert!(score <= score_max);
                    assert!(if dir == 1 { stop < price } else { stop > price });
                    open = Some((i, dir));
                    entries += 1;
                }
                Event::Exit { reason, .. } => {
                    let (at, _) = open.take().expect("exit without entry");
                    let held = i - at;
                    assert!(held >= 1 && held <= 4, "held {held} bars ({reason})");
                    if reason == "Hold 4 bars" {
                        assert_eq!(held, 4);
                    }
                    assert!(["SL", "Reversal", "Hold 4 bars", "Signal flip"].contains(&reason), "{reason}");
                }
                Event::Partial { .. } => panic!("no partials in this model"),
            }
        }
    }
    assert!(entries > 5, "synthetic market should trade, got {entries}");
    assert!(e.candidates as usize >= entries);
    assert!(e.status().ready);
    assert!(e.status().neighbours <= p.neighbors_count);
}

#[test]
fn dynamic_mode_exits_on_kernel_turns() {
    let mut e = Engine::new(Params { exit_mode: ExitMode::Dynamic, ..small() });
    let mut reasons = std::collections::BTreeSet::new();
    for b in bars(3000) {
        for ev in e.on_bar(b, true) {
            if let Event::Exit { reason, .. } = ev {
                reasons.insert(reason);
            }
        }
    }
    assert!(reasons.contains("Kernel exit"), "{reasons:?}");
    assert!(!reasons.contains("Hold 4 bars"));
}

#[test]
fn no_entries_when_not_allowed_and_force_close() {
    let mut e = Engine::new(small());
    for b in bars(1500) {
        assert!(e.on_bar(b, false).iter().all(|ev| !matches!(ev, Event::Entry { .. })));
    }
    assert!(e.rejects[2] > 0);
    assert!(e.force_close(8000.0, "Square-off").is_none());
}

#[test]
fn same_bars_same_signals() {
    let run = || {
        let mut e = Engine::new(small());
        bars(2000).into_iter().flat_map(|b| e.on_bar(b, true)).collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}
