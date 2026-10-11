//! Live state of one slot run (any strategy: WVAD; Sniper / SATS parked in backup/) for the web dashboard.
//! Strategy-specific content comes from the strategy: the header `title`, and either
//! its own `model_title` / `model_rows` panel or (SATS) the trend-model fields.
//!
//! The strategy never touches a `Board` directly: it pushes small updates through a
//! lock-free queue ([`super::dash_writer`]) and one writer thread owns the board,
//! applies them and publishes it to the dashboard Redis. At the end of the run the
//! board is printed once as text on stderr.
//!
//! JSON event logs go to stdout (a file); output errors are ignored everywhere so a
//! closed terminal or log can never crash the runner.
use serde::Serialize;
use std::collections::VecDeque;
use std::io::Write;

/// Events kept in the snapshot (newest first).
const EVENTS_KEPT: usize = 8;
/// Trades of this run kept in the snapshot (oldest dropped first).
const TRADES_KEPT: usize = 80;
/// Bars of the model chart kept in the snapshot.
const CHART_KEPT: usize = 240;
/// Events waiting for the writer, kept while the dashboard Redis is unreachable.
const PENDING_KEPT: usize = 500;

/// Writes one JSON event line to stdout; never panics on a broken pipe.
pub fn emit(value: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// Writes a human note to stderr; never panics.
pub fn note(text: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}

/// One trade this run opened: the model's levels and what happened to them, plus the
/// broker fills booked against it.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct TradeRow {
    /// IST time the entry signal was acted on (HH:MM:SS).
    pub opened: String,
    /// 1 long, -1 short.
    pub dir: i32,
    pub lots: u32,
    /// Model entry (signal bar close), stop and targets.
    pub entry: f64,
    pub stop: f64,
    pub tps: [f64; 3],
    /// Model grade / strength label.
    pub grade: String,
    /// Targets reached, e.g. "TP1 8866".
    pub hits: Vec<String>,
    pub closed: Option<String>,
    pub exit: Option<f64>,
    pub reason: Option<String>,
    /// Model gross R of the whole trade (no costs).
    pub model_r: Option<f64>,
    /// Average broker price of the lots opened for this trade.
    pub fill_entry: Option<f64>,
    /// Lots the broker has open for this trade now.
    pub open_lots: f64,
    /// Realised points from broker fills, summed over lots (no costs).
    pub realized_points: f64,
    #[serde(skip)]
    filled_lots: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct Board {
    pub mode: String,
    pub slot: String,
    pub instrument: String,
    pub square_off: String,
    pub exit_rule: String,
    pub redis_namespace: String,
    pub point_value: f64,
    pub lots: u32,
    /// Header title, set by the strategy runner (e.g. "WVAD v1.1.0 · 3m WVAD 20 · Sma 9 ...").
    pub title: String,
    /// Model panel: title and rows. Empty rows = the trend-model fields below.
    pub model_title: String,
    pub model_rows: Vec<(String, String)>,
    /// Model chart: series names and one entry per bar ("HH:MM", values), oldest first.
    pub model_chart_names: Vec<String>,
    pub model_chart: VecDeque<(String, Vec<f64>)>,
    /// Trades this run opened, oldest first.
    pub trades: VecDeque<TradeRow>,
    // feed
    pub last_price: Option<f64>,
    pub last_bar: Option<(String, f64)>,
    pub live_bars: u64,
    pub history_bars: u64,
    /// Bar length (ns) for the next-bar countdown; 0 hides it.
    pub bar_ns: i64,
    pub feed_fault: Option<String>,
    // trend model (SATS fills SuperTrend / TQI; `warmed` and `trend` are shared)
    pub warmed: bool,
    pub trend: i8,
    pub supertrend: Option<f64>,
    pub tqi: f64,
    pub regime: String,
    pub next_r: [f64; 3],
    // position (broker fills)
    pub position: f64,
    pub entry_avg: Option<f64>,
    pub sl: Option<f64>,
    /// Trigger of the SL-M resting at the broker, if any.
    pub exchange_stop: Option<f64>,
    pub tps: Option<[f64; 3]>,
    pub realized_points: f64,
    pub round_trips: u32,
    pub fills: u32,
    // control
    pub status: String,
    pub halted: Option<String>,
    /// Dashboard updates dropped because the queue was full (should stay 0).
    pub dropped: u64,
    pub(crate) events: VecDeque<String>,
    /// New events (epoch ms, text) not yet written to the dashboard event stream.
    #[serde(skip)]
    pub(crate) pending: VecDeque<(i64, String)>,
}

impl Board {
    pub fn event(&mut self, text: String) {
        let now = chrono::Utc::now();
        let stamp = now.with_timezone(&ist()).format("%H:%M:%S");
        self.events.push_front(format!("{stamp}  {text}"));
        self.events.truncate(EVENTS_KEPT);
        self.pending.push_back((now.timestamp_millis(), text));
        while self.pending.len() > PENDING_KEPT {
            self.pending.pop_front();
        }
    }

    /// A new trade this run opened (model entry and levels).
    pub fn open_trade(&mut self, dir: i32, lots: u32, entry: f64, stop: f64, tps: [f64; 3], grade: &str) {
        let opened = chrono::Utc::now().with_timezone(&ist()).format("%H:%M:%S").to_string();
        self.trades.push_back(TradeRow { opened, dir, lots, entry, stop, tps, grade: grade.into(), ..TradeRow::default() });
        while self.trades.len() > TRADES_KEPT {
            self.trades.pop_front();
        }
    }

    /// The model's open trade (the newest one not closed).
    fn open_row(&mut self) -> Option<&mut TradeRow> {
        self.trades.iter_mut().rev().find(|t| t.closed.is_none())
    }

    /// A target of the open trade was reached.
    pub fn trade_hit(&mut self, what: String) {
        if let Some(t) = self.open_row() {
            t.hits.push(what);
        }
    }

    /// The open trade ended in the model (stop, target, reversal, square-off, halt).
    pub fn close_trade(&mut self, reason: &str, exit: Option<f64>, model_r: Option<f64>) {
        let closed = chrono::Utc::now().with_timezone(&ist()).format("%H:%M:%S").to_string();
        if let Some(t) = self.open_row() {
            t.closed = Some(closed);
            t.reason = Some(reason.into());
            t.exit = exit;
            t.model_r = model_r;
        }
    }

    /// One bar of the model chart.
    pub fn chart_point(&mut self, names: &[&str], label: String, values: Vec<f64>) {
        if self.model_chart_names.len() != names.len() {
            self.model_chart_names = names.iter().map(|n| (*n).to_owned()).collect();
        }
        self.model_chart.push_back((label, values));
        while self.model_chart.len() > CHART_KEPT {
            self.model_chart.pop_front();
        }
    }

    /// The newest trade on side `dir` (the one a fill on that side belongs to).
    fn row_for(&mut self, dir: f64) -> Option<&mut TradeRow> {
        self.trades.iter_mut().rev().find(|t| f64::from(t.dir) == dir)
    }

    /// Books `lots` opened at `price` on side `dir` to that side's newest trade.
    fn book_open(&mut self, dir: f64, lots: f64, price: f64) {
        if let Some(t) = self.row_for(dir) {
            let cost = t.fill_entry.unwrap_or(price) * t.filled_lots + price * lots;
            t.filled_lots += lots;
            t.fill_entry = Some(cost / t.filled_lots);
            t.open_lots += lots;
        }
    }

    /// Applies one broker fill (`signed_qty` > 0 buys) to position, average entry and realised points.
    pub fn apply_fill(&mut self, signed_qty: f64, price: f64) {
        self.fills += 1;
        let before = self.position;
        let after = before + signed_qty;
        if before == 0.0 || before.signum() == signed_qty.signum() {
            // opening or adding
            let cost = self.entry_avg.unwrap_or(price) * before.abs() + price * signed_qty.abs();
            self.entry_avg = Some(cost / after.abs());
            self.book_open(after.signum(), signed_qty.abs(), price);
        } else {
            // reducing, closing, or a flip (one order through zero, e.g. +2 → −3)
            let closed = signed_qty.abs().min(before.abs());
            if let Some(entry) = self.entry_avg {
                let points = before.signum() * (price - entry) * closed;
                self.realized_points += points;
                if let Some(t) = self.row_for(before.signum()) {
                    t.realized_points += points;
                    t.open_lots = (t.open_lots - closed).max(0.0);
                }
            }
            if after != 0.0 && after.signum() != before.signum() {
                self.book_open(after.signum(), after.abs(), price);
            }
            if after == 0.0 {
                self.entry_avg = None;
                self.round_trips += 1;
                self.sl = None;
                self.tps = None;
            } else if after.signum() != before.signum() {
                // flip: the old trade is closed; the remainder opens the new one here
                self.entry_avg = Some(price);
                self.round_trips += 1;
            }
        }
        self.position = after;
    }

    /// "13:20  (in 3:41)" — the next bar close, bars aligned to the clock (5m → :00, :05, …).
    pub fn next_bar(&self, now: chrono::DateTime<chrono::FixedOffset>) -> String {
        if self.bar_ns <= 0 {
            return "—".into();
        }
        let now_ns = now.timestamp_nanos_opt().unwrap_or(0);
        let next = (now_ns / self.bar_ns + 1) * self.bar_ns;
        let left = (next - now_ns) / 1_000_000_000;
        let at = chrono::DateTime::from_timestamp_nanos(next).with_timezone(&ist());
        format!("{}  (in {}:{:02})", at.format("%H:%M"), left / 60, left % 60)
    }

    fn title(&self) -> String {
        if self.title.is_empty() { "Kite bot".into() } else { self.title.clone() }
    }

    pub fn unrealized_points(&self) -> Option<f64> {
        Some(self.position * (self.last_price? - self.entry_avg?))
    }

    /// Plain-text summary (ANSI colours), printed once on stderr when the run ends.
    pub fn render(&self, now: chrono::DateTime<chrono::FixedOffset>, square_off_ns: i64) -> String {
        let (g, r, y, b, d, x) = ("\x1b[32m", "\x1b[31m", "\x1b[33m", "\x1b[1m", "\x1b[2m", "\x1b[0m");
        let money = |pts: f64| {
            let c = if pts > 0.0 { g } else if pts < 0.0 { r } else { "" };
            format!("{c}{pts:+.1} pts  ₹{:+.0}{x}", pts * self.point_value)
        };
        let px = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.0}"));
        let left = (square_off_ns - now.timestamp_nanos_opt().unwrap_or(0)) / 1_000_000_000;
        let countdown = if left > 0 { format!("{}h {:02}m", left / 3600, (left % 3600) / 60) } else { "reached".into() };
        let status_col = match self.status.as_str() {
            "RUNNING" => g,
            "STARTING" | "SQUARED OFF" | "STOPPING" => y,
            _ => r,
        };
        let mode_col = if self.mode.starts_with("LIVE") { r } else { y };
        let trend = match self.trend {
            1 => format!("{g}▲ BULLISH{x}"),
            -1 => format!("{r}▼ BEARISH{x}"),
            _ => "—".into(),
        };
        let pos = if self.position > 0.0 {
            format!("{g}LONG {:.0} lot{x}", self.position)
        } else if self.position < 0.0 {
            format!("{r}SHORT {:.0} lot{x}", -self.position)
        } else {
            "FLAT".into()
        };
        let line = format!("{d}{}{x}", "─".repeat(64));
        let mut s = String::new();
        let mut row = |t: String| {
            s.push_str(&t);
            s.push('\n');
        };
        row(format!("{b} {}  ·  {}  ·  {}{x}", self.title(), self.instrument, self.slot));
        row(format!(" {mode_col}{b}{}{x}   status {status_col}{b}{}{x}   {} IST", self.mode, self.status, now.format("%H:%M:%S")));
        row(line.clone());
        row(format!(" Price        {b}{}{x}     last bar {}", px(self.last_price),
            self.last_bar.as_ref().map_or("—".into(), |(t, c)| format!("{t} close {c:.0}"))));
        row(format!(" Bars         history {} · live {}   next {}", self.history_bars, self.live_bars, self.next_bar(now)));
        row(format!(" Feed         {}",
            self.feed_fault.as_ref().map_or(format!("{g}OK{x}"), |f| format!("{r}FAULT: {f}{x}"))));
        row(line.clone());
        if self.model_rows.is_empty() {
            row(format!(" Trend        {trend}    SuperTrend {}", px(self.supertrend)));
            row(format!(" TQI          {:.2} ({})    warmed {}", self.tqi, self.regime, if self.warmed { "yes" } else { "no" }));
            row(format!(" TP1 at      {:.2} R   exit: {}", self.next_r[0], self.exit_rule));
        } else {
            row(format!(" Trend        {trend}    warmed {}", if self.warmed { "yes" } else { "no" }));
            for (k, v) in &self.model_rows {
                row(format!(" {k:<12} {v}"));
            }
            row(format!(" Exit         {}", self.exit_rule));
        }
        row(line.clone());
        row(format!(" Position     {pos}    entry {}", px(self.entry_avg)));
        row(format!(" Stop / TP1   {} / {}{}", px(self.sl), px(self.tps.map(|t| t[0])),
            self.exchange_stop.map_or(String::new(), |t| format!("   (SL-M {t:.0} at Zerodha)"))));
        row(format!(" Unrealised   {}", self.unrealized_points().map_or("—".into(), money)));
        row(format!(" Realised     {}   ({} round trips, {} fills)", money(self.realized_points), self.round_trips, self.fills));
        if !self.trades.is_empty() {
            let won = self.trades.iter().filter(|t| t.closed.is_some() && t.realized_points > 0.0).count();
            let closed = self.trades.iter().filter(|t| t.closed.is_some()).count();
            let r: f64 = self.trades.iter().filter_map(|t| t.model_r).sum();
            row(format!(" Trades       {} ({closed} closed, {won} won)   model {r:+.2} R", self.trades.len()));
        }
        row(line.clone());
        row(format!(" Square-off   {} IST  (in {countdown})   lots {}", self.square_off, self.lots));
        if let Some(h) = &self.halted {
            row(format!(" {r}{b}HALTED: {h}{x}"));
        }
        row(format!(" Redis run    {}", self.redis_namespace));
        if self.dropped > 0 {
            row(format!(" {y}Dashboard    dropped {} updates (queue full){x}", self.dropped));
        }
        row(line);
        row(format!("{b} Recent events{x}"));
        if self.events.is_empty() {
            row(format!(" {d}none yet{x}"));
        }
        for e in &self.events {
            row(format!(" {e}"));
        }
        s
    }
}

pub(crate) fn ist() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(19_800).expect("IST")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_track_average_entry_and_realised_points() {
        let mut b = Board { point_value: 10.0, ..Board::default() };
        b.apply_fill(1.0, 8790.0);
        assert_eq!((b.position, b.entry_avg), (1.0, Some(8790.0)));
        b.last_price = Some(8800.0);
        assert_eq!(b.unrealized_points(), Some(10.0));
        b.apply_fill(-1.0, 8805.0);
        assert_eq!((b.position, b.entry_avg, b.round_trips), (0.0, None, 1));
        assert_eq!(b.realized_points, 15.0);
        b.apply_fill(-1.0, 8800.0); // short
        b.apply_fill(1.0, 8810.0); // covered higher: loss
        assert_eq!(b.realized_points, 5.0);
    }

    #[test]
    fn a_flip_closes_the_old_trade_and_opens_the_new_one_at_the_flip_price() {
        let mut b = Board::default();
        b.apply_fill(3.0, 8800.0);
        b.apply_fill(-1.0, 8820.0); // TP1: +20 on 1 lot
        b.apply_fill(-5.0, 8810.0); // flip +2 → −3: +10 on 2 lots, short 3 from 8810
        assert_eq!(b.realized_points, 40.0);
        assert_eq!((b.position, b.entry_avg, b.round_trips), (-3.0, Some(8810.0), 1));
        b.apply_fill(3.0, 8790.0); // cover lower: +20 on 3 lots
        assert_eq!(b.realized_points, 100.0);
        assert_eq!((b.position, b.entry_avg, b.round_trips), (0.0, None, 2));
    }

    #[test]
    fn trades_collect_levels_targets_and_their_own_fills_through_a_flip() {
        let mut b = Board::default();
        b.open_trade(1, 3, 8800.0, 8780.0, [8820.0, 8840.0, 8860.0], "High");
        b.apply_fill(3.0, 8801.0);
        b.trade_hit("TP1 8820".into());
        b.apply_fill(-1.0, 8820.0); // +19 on 1 lot
        // reversal: the model closes the long and opens a short, one flip order fills
        b.close_trade("Reversal", Some(8810.0), Some(0.5));
        b.open_trade(-1, 3, 8810.0, 8830.0, [8790.0, 8770.0, 8750.0], "Low");
        b.apply_fill(-5.0, 8809.0); // closes 2 longs (+8 each), opens 3 shorts at 8809
        let long = &b.trades[0];
        assert_eq!((long.fill_entry, long.open_lots, long.realized_points), (Some(8801.0), 0.0, 35.0));
        assert_eq!((long.reason.as_deref(), long.hits.len(), long.model_r), (Some("Reversal"), 1, Some(0.5)));
        let short = &b.trades[1];
        assert_eq!((short.fill_entry, short.open_lots, short.closed.is_none()), (Some(8809.0), 3.0, true));
        b.apply_fill(3.0, 8790.0); // cover: +19 on 3 lots
        b.close_trade("TP3", Some(8790.0), Some(1.0));
        assert_eq!((b.trades[1].realized_points, b.trades[1].open_lots), (57.0, 0.0));
        assert_eq!(b.realized_points, 35.0 + 57.0, "per-trade points add up to the run total");
    }

    #[test]
    fn model_chart_keeps_the_newest_bars() {
        let mut b = Board::default();
        for i in 0..(CHART_KEPT + 5) {
            b.chart_point(&["WVAD", "Signal"], format!("{i}"), vec![i as f64, 0.0]);
        }
        assert_eq!(b.model_chart.len(), CHART_KEPT);
        assert_eq!(b.model_chart.front().map(|p| p.0.as_str()), Some("5"));
        assert_eq!(b.model_chart_names, vec!["WVAD", "Signal"]);
    }

    #[test]
    fn next_bar_counts_down_to_the_clock_aligned_close() {
        let b = Board { bar_ns: 300_000_000_000, ..Board::default() };
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-09T13:14:42+05:30").unwrap();
        assert_eq!(b.next_bar(now), "13:15  (in 0:18)");
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-09T13:15:00+05:30").unwrap();
        assert_eq!(b.next_bar(now), "13:20  (in 5:00)");
        assert_eq!(Board::default().next_bar(now), "—");
    }

    #[test]
    fn render_shows_the_essentials() {
        let mut b = Board {
            mode: "LIVE (real Zerodha orders)".into(),
            instrument: "CRUDEOILM26OCTFUT.MCX".into(),
            square_off: "23:15".into(),
            point_value: 10.0,
            status: "RUNNING".into(),
            dropped: 3,
            ..Board::default()
        };
        b.apply_fill(1.0, 8790.0);
        b.last_price = Some(8780.0);
        b.event("BUY 1 lot".into());
        let text = b.render(chrono::Utc::now().with_timezone(&ist()), 0);
        for needle in ["CRUDEOILM26OCTFUT.MCX", "LIVE", "LONG 1 lot", "₹-100", "BUY 1 lot", "Square-off", "dropped 3"] {
            assert!(text.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn events_keep_the_last_eight_and_queue_every_one_for_the_stream() {
        let mut b = Board::default();
        for i in 0..10 {
            b.event(format!("e{i}"));
        }
        assert_eq!(b.events.len(), 8);
        assert!(b.events[0].ends_with("e9"), "newest first");
        assert_eq!(b.pending.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), (0..10).map(|i| format!("e{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn snapshot_json_has_the_dashboard_fields_and_no_pending_queue() {
        let mut b = Board { slot: "crudeoilm-wvad-202610".into(), status: "RUNNING".into(), ..Board::default() };
        b.event("hello".into());
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["slot"], "crudeoilm-wvad-202610");
        assert_eq!(v["status"], "RUNNING");
        assert!(v["events"][0].as_str().unwrap().ends_with("hello"));
        assert!(v.get("pending").is_none());
    }
}
