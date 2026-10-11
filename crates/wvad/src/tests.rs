use crate::{
    Bar, Engine, Event, Params,
    filters::{Adaptive, Ema, Hma, Kalman, Llama, Rma, Sma, Vwma, Wma},
    params::Filter,
};

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

#[test]
fn moving_averages_match_pine_definitions() {
    let mut s = Sma::new(3);
    assert_eq!(s.update(1.0), None);
    assert_eq!(s.update(2.0), None);
    assert!(close(s.update(3.0), 2.0));
    assert!(close(s.update(7.0), 4.0));

    let mut e = Ema::new(3);
    assert!(close(e.update(2.0), 2.0), "ta.ema seeds with the first value");
    assert!(close(e.update(4.0), 3.0), "alpha 0.5");

    let mut r = Rma::new(2);
    assert_eq!(r.update(2.0), None);
    assert!(close(r.update(4.0), 3.0), "ta.rma seeds with the SMA");
    assert!(close(r.update(5.0), 4.0));

    let mut w = Wma::new(3);
    w.update(1.0);
    w.update(2.0);
    assert!(close(w.update(3.0), (1.0 + 4.0 + 9.0) / 6.0), "newest weighs most");

    let mut v = Vwma::new(2);
    assert_eq!(v.update(10.0, 1.0), None);
    assert!(close(v.update(20.0, 3.0), (10.0 + 60.0) / 4.0));
    assert_eq!(v.update(5.0, 0.0).map(|x| x.round()), Some(20.0), "(60 + 0) / 3");
    let mut zero = Vwma::new(1);
    assert_eq!(zero.update(5.0, 0.0), None, "no volume: na");
}

#[test]
fn hma_llama_and_kalman_follow_the_script() {
    // HMA(4): wma2, wma4, then wma of round(sqrt 4) = 2
    let mut h = Hma::new(4);
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let out: Vec<_> = xs.iter().map(|x| h.update(*x)).collect();
    assert_eq!(out[3], None, "needs one more value for the outer WMA");
    // on a straight line HMA is the line itself
    assert!(close(out[4], 5.0));

    let mut l = Llama::new(2);
    assert_eq!(l.update(1.0), None);
    assert_eq!(l.update(2.0), None, "needs src[len]");
    // sma(2,4) = 3, slope (4 - 1) / 2 = 1.5, × len / 2 = 1 → 4.5
    assert!(close(l.update(4.0), 4.5));

    let mut k = Kalman::new(1);
    // na source: error still advances, no estimate
    assert_eq!(k.update(None), None);
    // err 1 → pred 2 → gain 2/3 → err 2/3; first real value seeds the estimate
    assert!(close(k.update(Some(10.0)), 10.0));
    // err after the seed: 3/8 × 5/3 = 5/8; pred 13/8, gain 13/21: 10 + 13/21 × (18 - 10)
    assert!(close(k.update(Some(18.0)), 10.0 + 13.0 / 21.0 * 8.0));
}

#[test]
fn adaptive_filters_skip_the_leading_na() {
    for f in [Filter::None, Filter::Sma, Filter::Ema, Filter::Rma, Filter::DoubleWma, Filter::TripleVwma, Filter::Hma, Filter::Llama, Filter::Kalman] {
        let mut a = Adaptive::new(f, 3);
        assert_eq!(a.update(None, 10.0), None, "{f:?}");
        let mut last = None;
        for i in 0..20 {
            last = a.update(Some(100.0 + i as f64), 10.0);
        }
        assert!(last.is_some_and(f64::is_finite), "{f:?} produces values");
    }
}

/// Deterministic wavy market, 09:00-23:30 IST, 3m bars, volume always > 0.
fn market(days: i64) -> Vec<Bar> {
    let day0 = 1_788_220_800; // 2026-09-01 00:00 UTC
    let mut out = Vec::new();
    let mut px = 8000.0_f64;
    let mut n = 0_i64;
    for day in 0..days {
        let open = day0 + day * 86_400 + 3 * 3600 + 1800; // 09:00 IST
        for k in 0..290_i64 {
            let drift = (n as f64 / 17.0).sin() * 9.0 + (n as f64 / 5.0).cos() * 4.0;
            let o = px;
            let c = (px + drift).round();
            out.push(Bar { start: open + k * 180, open: o, high: o.max(c) + 3.0, low: o.min(c) - 3.0, close: c, volume: 100.0 + ((n * 37) % 90) as f64 });
            px = c;
            n += 1;
        }
    }
    out
}

#[test]
fn entries_use_the_crossover_close_and_atr_levels() {
    let p = Params::default();
    let mut e = Engine::new(p.clone());
    let mut entries = 0;
    let mut open_dir = 0;
    for b in market(6) {
        for ev in e.on_bar(b, true) {
            match ev {
                Event::Entry { dir, price, stop, tp1, tp2, tp3, .. } => {
                    entries += 1;
                    assert_eq!(price, b.close, "entry at the signal bar close");
                    let risk = (price - stop) * dir as f64;
                    assert!((risk - e.status().atr * p.sl_mult).abs() < 1e-9, "stop = ATR × SL mult");
                    for (tp, m) in [(tp1, 1.0), (tp2, 2.0), (tp3, 3.0)] {
                        assert!(((tp - price) * dir as f64 - risk * m).abs() < 1e-9);
                    }
                    assert_eq!(e.status().bias, dir, "WVAD is on the signal side of its line");
                    assert_ne!(open_dir, dir, "never a second trade in the same direction");
                    open_dir = dir;
                }
                Event::Exit { reason, .. } => {
                    assert!(["SL", "TP3", "Reversal"].contains(&reason), "{reason}");
                    open_dir = 0;
                }
                Event::Partial { .. } => panic!("whole-position default has no partials"),
            }
        }
    }
    assert!(entries > 10, "synthetic market should trade, got {entries}");
    assert!(e.candidates as usize >= entries);
}

#[test]
fn no_signal_before_the_warm_up_and_none_when_entries_are_closed() {
    let mut e = Engine::new(Params::default());
    for (i, b) in market(3).into_iter().enumerate() {
        let evs = e.on_bar(b, false);
        assert!(evs.is_empty(), "bar {i}: entries closed");
        if i < 200 {
            assert!(!e.status().ready);
        }
    }
    assert!(e.rejects[2] > 0, "crossovers rejected as outside the entry window");
}

#[test]
fn stop_is_taken_first_and_partials_follow_the_fractions() {
    let p = Params { tp1_close_fraction: 1.0 / 3.0, tp2_close_fraction: 1.0 / 3.0, ..Params::default() };
    let mut e = Engine::new(p);
    let bars = market(6);
    let mut partials = 0;
    for b in bars {
        for ev in e.on_bar(b, true) {
            if let Event::Partial { fraction, .. } = ev {
                assert!((fraction - 1.0 / 3.0).abs() < 1e-12);
                partials += 1;
            }
            if let Event::Exit { fraction, gross_r, .. } = ev {
                assert!(fraction > 0.0 && fraction <= 1.0 + 1e-12);
                assert!(gross_r.is_finite());
            }
        }
    }
    assert!(partials > 0, "some trade reaches TP1");

    // hand-made trade: a bar through both the stop and TP1 exits at the stop
    let mut e = Engine::new(Params::default());
    e.trade = Some(crate::Trade {
        dir: 1, entry_bar: 0, entry: 100.0, initial_stop: 90.0, stop: 90.0, risk: 10.0, tp1: 110.0, tp2: 120.0, tp3: 130.0,
        hit1: false, hit2: false, hit3: false, ambiguous: false, remaining: 1.0, gross_r: 0.0,
    });
    e.on_bar(Bar { start: 1_788_220_800, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0 }, false);
    let evs = e.on_bar(Bar { start: 1_788_220_980, open: 100.0, high: 111.0, low: 89.0, close: 100.0, volume: 1.0 }, false);
    match evs.as_slice() {
        [Event::Exit { reason: "SL", price, gross_r, ambiguous: true, .. }] => {
            assert_eq!(*price, 90.0);
            assert!((gross_r + 1.0).abs() < 1e-12);
        }
        other => panic!("expected an SL exit, got {other:?}"),
    }
}

#[test]
fn tp3_closes_and_the_stop_stays_put_without_step_stop() {
    let mut e = Engine::new(Params::default());
    e.trade = Some(crate::Trade {
        dir: -1, entry_bar: 0, entry: 100.0, initial_stop: 110.0, stop: 110.0, risk: 10.0, tp1: 90.0, tp2: 80.0, tp3: 70.0,
        hit1: false, hit2: false, hit3: false, ambiguous: false, remaining: 1.0, gross_r: 0.0,
    });
    let b = |start: i64, low: f64| Bar { start, open: 100.0, high: 101.0, low, close: 95.0, volume: 1.0 };
    e.on_bar(b(1_788_220_800, 99.0), false);
    assert!(e.on_bar(b(1_788_220_980, 85.0), false).is_empty(), "TP1 is only a mark");
    assert_eq!(e.trade.as_ref().map(|t| (t.hit1, t.stop)), Some((true, 110.0)));
    match e.on_bar(b(1_788_221_160, 69.0), false).as_slice() {
        [Event::Exit { reason: "TP3", price, gross_r, .. }] => {
            assert_eq!(*price, 70.0);
            assert!((gross_r - 3.0).abs() < 1e-12);
        }
        other => panic!("expected TP3, got {other:?}"),
    }
}

#[test]
fn htf_and_adx_filters_only_ever_remove_trades() {
    let entries = |p: Params| {
        let mut e = Engine::new(p);
        let n = market(6).into_iter().flat_map(|b| e.on_bar(b, true)).filter(|ev| matches!(ev, Event::Entry { .. })).count();
        (n, e.rejects)
    };
    let (base, _) = entries(Params::default());
    let (htf, rejects) = entries(Params { use_htf: true, ..Params::default() });
    assert!(htf < base && rejects[4] > 0, "HTF filter: {htf} of {base}");
    let (adx0, _) = entries(Params { use_adx: true, adx_threshold: 0.0, ..Params::default() });
    assert_eq!(adx0, base, "ADX >= 0 passes everything");
    let (adx100, rejects) = entries(Params { use_adx: true, adx_threshold: 100.0, ..Params::default() });
    assert!(adx100 == 0 && rejects[3] > 0, "ADX >= 100 passes nothing");
}

#[test]
fn params_validate_and_round_trip() {
    let p = Params::default();
    p.validate().unwrap();
    assert_eq!(p.min_warmup(), 20 + 9 + 9 + 1);
    let back: Params = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back, p);
    let f: Params = serde_json::from_str(r#"{"adapt_filter":"Kalman Filter","use_htf":true,"htf_minutes":60}"#).unwrap();
    assert_eq!(f.adapt_filter, Filter::Kalman);
    f.validate().unwrap();
    assert!(Params { use_htf: true, htf_minutes: 4, ..Params::default() }.validate().is_err());
    assert!(Params { warmup_bars: 10, ..Params::default() }.validate().is_err());
    assert!(Params { tp2_r: 0.5, ..Params::default() }.validate().is_err());
}
