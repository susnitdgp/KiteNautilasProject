//! Machine Learning: Lorentzian Classification (crate `lorentzian`) plugged into the shared
//! bar-model runner. Backtest only for now (`native-lorentzian-backtest`); no paper / live
//! commands are wired.
use super::{
    bar_model::{BarModel, Levels},
    slot_config::ModelParams,
};
use lorentzian::{Engine, Params};
use std::collections::BTreeMap;
use wvad::{Bar, Event};

#[cfg(test)]
pub type LorentzianConfig = super::slot_config::SlotConfig<Params>;

impl ModelParams for Params {
    const STRATEGY: &'static str = "lorentzian";
    const NAME: &'static str = "LDC";
    const MODEL_TITLE: &'static str = "Lorentzian Classification";
    const SOURCE_VERSION: &'static str = lorentzian::VERSION;
    fn timeframe_minutes(&self) -> u32 {
        self.timeframe_minutes
    }
    fn validate(&self) -> Result<(), String> {
        Params::validate(self)
    }
    fn fractions(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
    fn with_fractions(&self, tp1: f64, tp2: f64) -> Self {
        // no take-profit levels in this model: partial exits cannot be honoured
        assert!(tp1 == 0.0 && tp2 == 0.0, "lorentzian has no targets: set tp1_lots and tp2_lots to 0");
        self.clone()
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
        "fixed ATR stop"
    }
    fn backtest_warmup_days(&self) -> i64 {
        self.history_days
    }
}

impl BarModel for Engine {
    fn on_bar(&mut self, b: Bar, entries_allowed: bool) -> Vec<Event> {
        Engine::on_bar(self, b, entries_allowed)
    }
    fn force_close(&mut self, price: f64, reason: &'static str) -> Option<Event> {
        Engine::force_close(self, price, reason)
    }
    fn mark_target(&mut self, _level: u8) -> Vec<Event> {
        Vec::new()
    }
    fn levels(&self) -> Option<Levels> {
        self.trade.as_ref().map(|t| Levels {
            dir: t.dir,
            entry: t.entry,
            stop: t.stop,
            initial_stop: t.stop,
            // no targets: NaN never compares as reached
            tps: [f64::NAN; 3],
            hits: [false; 3],
            remaining: 1.0,
        })
    }
    fn ready(&self) -> bool {
        self.status().ready
    }
    fn trend(&self) -> i32 {
        self.status().signal
    }
    fn model_rows(&self) -> Vec<(String, String)> {
        let s = self.status();
        let p = self.params();
        vec![
            (format!("Prediction (k {})", p.neighbors_count), format!("{:+.0} · signal {:+}", s.prediction, s.signal)),
            ("Kernel".into(), format!("{:.1} ({})", s.kernel, match s.kernel_slope { 1 => "rising", -1 => "falling", _ => "flat" })),
            ("Filters".into(), format!("{} · regime {:.2} · ADX {:.0}", if s.filters { "pass" } else { "block" }, s.regime, s.adx)),
        ]
    }
    fn bar_log(&self) -> serde_json::Value {
        let s = self.status();
        serde_json::json!({"prediction": s.prediction, "signal": s.signal, "kernel": (s.kernel * 10.0).round() / 10.0,
            "kernel_slope": s.kernel_slope, "filters": s.filters})
    }
    fn describe(&self) -> String {
        self.params().describe()
    }
    fn candidates(&self) -> u32 {
        self.candidates
    }
    fn chart(&self) -> Option<(&'static [&'static str], Vec<f64>)> {
        let s = self.status();
        s.kernel.is_finite().then(|| (&["Kernel"][..], vec![s.kernel]))
    }
    fn rejected(&self) -> BTreeMap<String, u32> {
        self.rejects
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(code, n)| (lorentzian::engine::reject_reason(code as u8).to_owned(), *n))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHIPPED: &str = include_str!("../../../../config/lorentzian-crudeoilm.json");
    #[test]
    fn shipped_config_is_valid_and_uses_the_script_defaults() {
        let c: LorentzianConfig = serde_json::from_str(SHIPPED).unwrap();
        c.validate().unwrap();
        assert_eq!((c.strategy.as_str(), c.bar_minutes, c.tp1_lots, c.tp2_lots), ("lorentzian", 5, 0, 0));
        let d = Params::default();
        let p = &c.params;
        assert_eq!((p.neighbors_count, p.max_bars_back, p.feature_count, p.features), (d.neighbors_count, d.max_bars_back, d.feature_count, d.features));
        assert_eq!((p.use_volatility_filter, p.use_regime_filter, p.regime_threshold, p.use_kernel_filter), (true, true, -0.1, true));
        let _ = c.model();
    }
    #[test]
    #[should_panic(expected = "no targets")]
    fn partial_lots_are_refused() {
        let mut c: LorentzianConfig = serde_json::from_str(SHIPPED).unwrap();
        c.tp1_lots = 1;
        let _ = c.model();
    }
}
