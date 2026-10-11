//! Williams Variable A/D Pressure [MarkitTick] (crate `wvad`) plugged into the shared
//! bar-model runner.
use super::{
    bar_model::{BarModel, Levels},
    slot_config::ModelParams,
};
use std::collections::BTreeMap;
use wvad::{Bar, Engine, Event, Params};

#[cfg(test)]
pub type WvadConfig = super::slot_config::SlotConfig<Params>;

impl ModelParams for Params {
    const STRATEGY: &'static str = "wvad";
    const NAME: &'static str = "WVAD";
    const MODEL_TITLE: &'static str = "Williams Variable A/D Pressure";
    const SOURCE_VERSION: &'static str = wvad::VERSION;
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
        self.warmup_bars + 20
    }
    fn label(&self) -> String {
        self.describe()
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
        self.status().bias
    }
    fn model_rows(&self) -> Vec<(String, String)> {
        let s = self.status();
        let p = self.params();
        let htf = match s.htf {
            None if !p.use_htf => "off".to_owned(),
            None => "waiting".to_owned(),
            Some(1) => "bull".to_owned(),
            Some(-1) => "bear".to_owned(),
            Some(_) => "flat".to_owned(),
        };
        vec![
            (format!("WVAD {} / signal {}", p.wvad_len, p.signal_len), format!("{} / {}  (hist {})", compact(s.wvad), compact(s.signal), compact(s.hist))),
            ("Strength / vol".into(), format!("{:.0}% · vol ×{:.1}", s.strength * 100.0, s.vol_ratio)),
            ("ADX / HTF".into(), format!("{:.0}{} · {htf}", s.adx, if p.use_adx { format!(" (min {})", p.adx_threshold) } else { String::new() })),
        ]
    }
    fn bar_log(&self) -> serde_json::Value {
        let s = self.status();
        serde_json::json!({"bias": s.bias, "wvad": s.wvad.round(), "signal": s.signal.round(),
            "strength": (s.strength * 100.0).round(), "adx": (s.adx * 10.0).round() / 10.0, "htf": s.htf})
    }
    fn describe(&self) -> String {
        self.params().describe()
    }
    fn candidates(&self) -> u32 {
        self.candidates
    }
    fn rejected(&self) -> BTreeMap<String, u32> {
        self.rejects
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(code, n)| (wvad::engine::reject_reason(code as u8).to_owned(), *n))
            .collect()
    }
}

/// 1234567 → "1.2M", like the script's dashboard numbers.
fn compact(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let a = v.abs();
    let s = if v < 0.0 { "-" } else { "" };
    if a >= 1e9 {
        format!("{s}{:.1}B", a / 1e9)
    } else if a >= 1e6 {
        format!("{s}{:.1}M", a / 1e6)
    } else if a >= 1e3 {
        format!("{s}{:.1}K", a / 1e3)
    } else {
        format!("{s}{a:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHIPPED: &str = include_str!("../../../../config/wvad-crudeoilm.json");
    #[test]
    fn shipped_config_is_valid_and_matches_the_script_defaults() {
        let c: WvadConfig = serde_json::from_str(SHIPPED).unwrap();
        c.validate().unwrap();
        assert_eq!((c.strategy.as_str(), c.bar_minutes, c.lots, c.tp1_lots, c.tp2_lots), ("wvad", 3, 3, 1, 1));
        let p = &c.params;
        let defaults = Params::default();
        assert_eq!(
            (p.wvad_len, p.signal_len, p.adapt_filter, p.adapt_filter_len, p.use_adx, p.use_htf, p.atr_len, p.sl_mult),
            (defaults.wvad_len, defaults.signal_len, defaults.adapt_filter, defaults.adapt_filter_len, false, false, 14, 1.5),
            "the script's default inputs"
        );
        assert_eq!((p.tp1_r, p.tp2_r, p.tp3_r, p.step_stop), (1.0, 2.0, 3.0, false));
        let e = c.engine_params();
        assert!((e.tp1_close_fraction - 1.0 / 3.0).abs() < 1e-12);
        let mut wrong = c.clone();
        wrong.strategy = "sniper".into();
        assert!(wrong.validate().is_err());
    }
    #[test]
    fn compact_numbers() {
        assert_eq!(compact(1_234_567.0), "1.2M");
        assert_eq!(compact(-2_500.0), "-2.5K");
        assert_eq!(compact(12.4), "12");
        assert_eq!(compact(f64::NAN), "—");
    }
}
