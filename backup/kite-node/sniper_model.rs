//! Precision Sniper v2.1.0 (crate `sniper`) plugged into the shared bar-model runner.
use super::{
    bar_model::{BarModel, Levels},
    slot_config::ModelParams,
};
use sniper::{Bar, Engine, Event, Params};
use std::collections::BTreeMap;

#[cfg(test)]
pub type SniperConfig = super::slot_config::SlotConfig<Params>;

impl ModelParams for Params {
    const STRATEGY: &'static str = "sniper";
    const NAME: &'static str = "SNIPER";
    const MODEL_TITLE: &'static str = "Precision Sniper";
    const SOURCE_VERSION: &'static str = sniper::PORT_VERSION;
    fn timeframe_minutes(&self) -> u32 {
        self.timeframe_minutes
    }
    fn validate(&self) -> Result<(), String> {
        Params::validate(self)
    }
    fn fractions(&self) -> (f64, f64) {
        (self.tp1_close_fraction, self.tp2_close_fraction)
    }
    fn with_fractions(&self, tp1: f64, tp2: f64) -> Self {
        Self { tp1_close_fraction: tp1, tp2_close_fraction: tp2, ..self.clone() }
    }
    fn build(&self) -> Box<dyn BarModel> {
        Box::new(Engine::new(self.clone()))
    }
    fn warmup_needed(&self) -> usize {
        let r = self.resolve();
        (r.trend * self.warmup_mult).max(r.atr + 42) + 20
    }
    fn label(&self) -> String {
        format!("{:?}", self.resolve().preset)
    }
    fn stop_rule(&self) -> &'static str {
        if self.step_stop { "step stop" } else { "fixed stop" }
    }
}

impl BarModel for Engine {
    fn on_bar(&mut self, b: Bar, entries_allowed: bool) -> Vec<Event> {
        Engine::on_bar(self, b, entries_allowed)
    }
    fn force_close(&mut self, price: f64, reason: &'static str) -> Option<Event> {
        Engine::force_close(self, price, reason)
    }
    fn mark_target(&mut self, level: u8) -> Vec<Event> {
        Engine::mark_target(self, level)
    }
    fn levels(&self) -> Option<Levels> {
        self.trade.as_ref().map(|t| Levels {
            dir: t.dir,
            entry: t.entry,
            stop: t.stop,
            initial_stop: t.initial_stop,
            tps: [t.tp1, t.tp2, t.tp3],
            hits: [t.hit1, t.hit2, t.hit3],
            remaining: t.remaining,
        })
    }
    fn ready(&self) -> bool {
        self.status().ready
    }
    fn trend(&self) -> i32 {
        self.status().trend_dir
    }
    fn model_rows(&self) -> Vec<(String, String)> {
        let s = self.status();
        let r = self.resolved();
        vec![
            (format!("EMA {}/{}", r.fast, r.slow), format!("{:.0} / {:.0}  (trend {:.0})", s.ema_fast, s.ema_slow, s.ema_trend)),
            ("Score".into(), format!("bull {:.0} · bear {:.0} of {:.0}", s.bull_score, s.bear_score, s.score_max)),
            ("ADX / RSI".into(), format!("{:.0} / {:.0}  {}{}", s.adx, s.rsi, s.regime, if s.high_vol { " · high vol" } else { "" })),
        ]
    }
    fn bar_log(&self) -> serde_json::Value {
        let s = self.status();
        serde_json::json!({"trend": s.trend_dir, "bull": s.bull_score, "bear": s.bear_score, "adx": (s.adx * 10.0).round() / 10.0})
    }
    fn describe(&self) -> String {
        format!("{:?}", self.resolved())
    }
    fn candidates(&self) -> u32 {
        self.candidates
    }
    fn rejected(&self) -> BTreeMap<String, u32> {
        self.rejects
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(code, n)| (sniper::engine::reject_reason(code as u8).to_owned(), *n))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::slot_config::Blackout;
    use super::*;
    use chrono::NaiveTime;
    const SHIPPED: &str = include_str!("../../../../config/sniper-crudeoilm.json");
    #[test]
    fn shipped_config_is_the_three_lot_live_setup() {
        let c: SniperConfig = serde_json::from_str(SHIPPED).unwrap();
        c.validate().unwrap();
        assert_eq!((c.lots, c.tp1_lots, c.tp2_lots, c.bar_minutes), (3, 1, 1, 3));
        let p = c.engine_params();
        assert!((p.tp1_close_fraction - 1.0 / 3.0).abs() < 1e-12 && (p.tp2_close_fraction - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(p.resolve().preset, sniper::params::Preset::Aggressive);
        assert_eq!(p.grade_filter, sniper::params::GradeFilter::AOrBetter);
        let mut bad = c.clone();
        bad.tp2_lots = 2;
        assert!(bad.validate().is_err(), "TP1 + TP2 must leave a lot for TP3");
        let mut wrong = c.clone();
        wrong.strategy = "wvad".into();
        assert!(wrong.validate().is_err(), "strategy name must match the params");
    }
    #[test]
    fn entry_window_and_blackout() {
        let c: SniperConfig = serde_json::from_str(SHIPPED).unwrap();
        // 2026-10-09 IST times as epoch seconds
        let at = |h: u32, m: u32| chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap().and_hms_opt(h, m, 0).unwrap().and_utc().timestamp() - 19_800;
        assert!(c.entries_allowed_at(at(10, 0)));
        assert!(c.entry_blackouts.is_empty(), "shipped without blackouts (2.26.2)");
        assert!(c.entries_allowed_at(at(18, 0)), "no US data blackout");
        let mut b = c.clone();
        b.entry_blackouts = vec![Blackout { from: NaiveTime::from_hms_opt(17, 30, 0).unwrap(), to: NaiveTime::from_hms_opt(19, 30, 0).unwrap() }];
        assert!(!b.entries_allowed_at(at(18, 0)), "blackout blocks entries");
        assert!(b.entries_allowed_at(at(19, 30)), "blackout ends at 19:30");
        let rules = b.rules();
        assert_eq!(rules.entries_allowed_at(at(18, 0)), b.entries_allowed_at(at(18, 0)), "live rules agree with the config");
        assert!(!c.entries_allowed_at(at(23, 0)));
        assert!(c.square_off_due(at(23, 15)) && !c.square_off_due(at(23, 12)));
        assert!(rules.square_off_due(at(23, 15)) && !rules.square_off_due(at(23, 12)));
    }
}
