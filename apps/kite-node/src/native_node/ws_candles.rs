//! Live OHLCV (3-minute by default) from Kite full WebSocket packets.
//!
//! kite-node 2.32.0 (pending #2 and #7b):
//! * each trade goes into the bar its **last-trade time** falls in (the exchange's and
//!   TradingView's rule), not the bar of the packet's send time;
//! * a bar is closed by the **clock** (`FeedEvent::Clock`), `BAR_CLOSE_GRACE_MS` after its
//!   boundary, not by the next packet: late-stamped trades of that bar still land in it and
//!   every bar closes at the same moment, also on a quiet tape;
//! * a packet without a new trade (same last-trade time, no new volume) changes no bar;
//! * a trade that arrives after its bar was closed adds its volume to the next bar and is
//!   counted (`late_trades`) instead of stopping the run;
//! * a bar period without any trade produces no bar, as on Kite's and TradingView's charts.
//!
//! The Historical API is used for warm-up and the startup gap only; reconnections, volume
//! resets and invalid packets still fail closed.
use anyhow::{Result, ensure};
use chrono::{TimeZone, Utc};
use kite_adapter::{http::historical::Candle, mapping::market_data::Snapshot};
use std::collections::VecDeque;

#[allow(dead_code)] // 3-minute default, used by tests
const DEFAULT_STEP: i64 = 180;
/// How long after its boundary a bar is closed (ms).
pub const BAR_CLOSE_GRACE_MS: i64 = 1_000;

#[derive(Debug, Default)]
pub struct Aggregator {
    /// Bars past their boundary, waiting out the grace (oldest first; normally at most one).
    closing: VecDeque<Current>,
    /// The bar of the latest trade.
    current: Option<Current>,
    /// End (epoch s) of the last bar closed; a trade at or before it is late.
    last_close: i64,
    started: bool,
    last_volume: Option<u32>,
    generation: Option<u32>,
    last_exchange_ts: Option<i64>,
    last_trade_ts: Option<i64>,
    step: i64,
    grace_ms: i64,
    /// `(warm-up close, first full stream bar start)` in epoch seconds, set on the
    /// first tick when candles between them can never come from the stream.
    gap: Option<(i64, i64)>,
    /// Volume of trades that arrived after their bar was closed, added to the next bar.
    carry_volume: u64,
    late_trades: u64,
}

#[derive(Debug)]
struct Current {
    start: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,
    oi: u64,
    /// The first bar of a run, joined part-way: never handed out (the backfill supplies it).
    partial: bool,
}

impl Current {
    fn new(start: i64, price: f64, volume: u64, oi: u64, partial: bool) -> Self {
        Self {
            start,
            open: price,
            high: price,
            low: price,
            close: price,
            volume,
            oi,
            partial,
        }
    }
    fn add(&mut self, price: f64, volume: u64, oi: u64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume += volume;
        self.oi = oi;
    }
}

impl Aggregator {
    #[allow(dead_code)] // 3-minute default, used by tests
    pub fn new(last_close_ns: u64) -> Self {
        Self::with_step(last_close_ns, DEFAULT_STEP)
    }
    /// Candle length in seconds (300 for 5-minute bars).
    pub fn with_step(last_close_ns: u64, step_seconds: i64) -> Self {
        assert!(step_seconds > 0 && 86_400 % step_seconds == 0, "invalid candle step");
        Self {
            last_close: (last_close_ns / 1_000_000_000) as i64,
            step: step_seconds,
            grace_ms: BAR_CLOSE_GRACE_MS,
            ..Self::default()
        }
    }
    fn bucket(&self, ts: i64) -> i64 {
        ts.div_euclid(self.step) * self.step
    }
    fn due_ms(&self, start: i64) -> i64 {
        (start + self.step) * 1_000 + self.grace_ms
    }
    /// Trades seen after their bar was closed (volume carried into the next bar).
    pub fn late_trades(&self) -> u64 {
        self.late_trades
    }

    /// Takes one full packet into the open bars. Never hands a bar out: `close_due` does.
    pub fn observe(&mut self, s: &Snapshot) -> Result<()> {
        ensure!(s.source_fresh, "stale WebSocket packet");
        let ts = i64::from(
            s.exchange_timestamp
                .ok_or_else(|| anyhow::anyhow!("missing exchange timestamp"))?,
        );
        let received = s.received_at_utc.timestamp();
        if let Some(prev) = self.last_exchange_ts {
            ensure!(ts >= prev, "out-of-order exchange timestamp");
        }
        ensure!(
            (-2..=10).contains(&(received - ts)),
            "stale/future exchange timestamp"
        );
        let full = s.raw.as_ref().and_then(|r| r.full.as_ref());
        ensure!(full.is_some(), "not a full WebSocket packet");
        // when the trade happened (1 s resolution); the packet time if Kite left it out
        let trade_ts = full
            .and_then(|f| f.last_trade_timestamp)
            .map(i64::from)
            .unwrap_or(ts);
        ensure!(trade_ts <= ts + 2, "last trade time after the packet time");
        let price: f64 = s.ltp.parse()?;
        ensure!(
            price.is_finite() && price > 0.0,
            "invalid last traded price"
        );
        if let Some(g) = self.generation {
            ensure!(
                g == s.connection_generation,
                "WebSocket generation changed: candle continuity unverified"
            );
        }
        self.generation = Some(s.connection_generation);
        let cumulative = s
            .cumulative_volume
            .ok_or_else(|| anyhow::anyhow!("missing cumulative volume"))?;
        let delta = match self.last_volume {
            Some(old) if cumulative >= old => u64::from(cumulative - old),
            Some(_) => {
                anyhow::bail!("cumulative volume reset: session/reconnect must be reconciled")
            }
            None => 0,
        };
        self.last_volume = Some(cumulative);
        self.last_exchange_ts = Some(ts);
        let oi = u64::from(s.open_interest.unwrap_or(0));

        if !self.started {
            self.started = true;
            self.last_trade_ts = Some(trade_ts);
            // The first bar is the packet's own, joined part-way unless exactly on its
            // boundary; candles before it come from the backfill (`take_gap`).
            let bucket = self.bucket(ts);
            ensure!(bucket >= self.last_close, "WebSocket tick predates warmup");
            let partial = ts > bucket;
            let first_full = if partial { bucket + self.step } else { bucket };
            if first_full > self.last_close {
                self.gap = Some((self.last_close, first_full));
            }
            self.current = Some(Current::new(bucket, price, 0, oi, partial));
            return Ok(());
        }

        let new_trade = delta > 0 || self.last_trade_ts.is_none_or(|p| trade_ts > p);
        self.last_trade_ts = Some(self.last_trade_ts.map_or(trade_ts, |p| p.max(trade_ts)));
        if !new_trade {
            // depth/quote refresh only: the bars keep their prices and volume
            if let Some(c) = self.current.as_mut() {
                c.oi = oi;
            }
            return Ok(());
        }
        let bucket = self.bucket(trade_ts);
        if bucket + self.step <= self.last_close {
            // its bar is already closed: keep the volume for the next bar
            self.carry_volume += delta;
            self.late_trades += 1;
            return Ok(());
        }
        if let Some(c) = self.closing.iter_mut().find(|c| c.start == bucket) {
            // a late-stamped trade of the bar inside its grace window
            c.add(price, delta, oi);
            return Ok(());
        }
        match self.current.as_ref().map(|c| c.start) {
            Some(start) if start == bucket => {
                self.current.as_mut().expect("current").add(price, delta, oi);
            }
            Some(start) if bucket > start => {
                // the current bar's period is over: it waits out the grace in `closing`
                let done = self.current.take().expect("current");
                self.closing.push_back(done);
                self.start_bar(bucket, price, delta, oi);
            }
            Some(_) => {
                // older than the current bar with no bar of its own left open: late
                self.carry_volume += delta;
                self.late_trades += 1;
            }
            None => self.start_bar(bucket, price, delta, oi),
        }
        Ok(())
    }

    fn start_bar(&mut self, start: i64, price: f64, volume: u64, oi: u64) {
        let volume = volume + std::mem::take(&mut self.carry_volume);
        self.current = Some(Current::new(start, price, volume, oi, false));
    }

    /// Bars whose period and grace are over at `now_ms` (epoch ms), oldest first.
    /// Called on every clock event and after every packet.
    pub fn close_due(&mut self, now_ms: i64) -> Vec<Candle> {
        let mut out = Vec::new();
        while let Some(start) = self.closing.front().map(|c| c.start) {
            if now_ms < self.due_ms(start) {
                break;
            }
            let c = self.closing.pop_front().expect("closing bar");
            self.finish(c, &mut out);
        }
        if self.closing.is_empty()
            && let Some(start) = self.current.as_ref().map(|c| c.start)
            && now_ms >= self.due_ms(start)
        {
            // no trade in the next period yet: the bar still closes on time
            let c = self.current.take().expect("current");
            self.finish(c, &mut out);
        }
        out
    }

    fn finish(&mut self, c: Current, out: &mut Vec<Candle>) {
        self.last_close = self.last_close.max(c.start + self.step);
        if c.partial {
            return;
        }
        out.push(Candle {
            timestamp: Utc
                .timestamp_opt(c.start, 0)
                .single()
                .expect("valid timestamp")
                .to_rfc3339(),
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            oi: c.oi,
        });
    }

    /// The startup gap to backfill from history, reported once.
    pub fn take_gap(&mut self) -> Option<(i64, i64)> {
        self.gap.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A full packet sent at `ts` for a trade at `trade_ts` (both epoch s).
    fn packet(ts: i64, trade_ts: i64, price: i32, volume: u32, generation: u32) -> Snapshot {
        let mut s = super::super::synthetic::full_snapshot_for(
            145894407,
            price * 100,
            (ts as u64) * 1_000_000_000,
            generation,
        );
        s.received_at_utc = Utc.timestamp_opt(ts, 0).single().unwrap();
        s.exchange_timestamp = Some(ts as u32);
        if let Some(full) = s.raw.as_mut().and_then(|r| r.full.as_mut()) {
            full.last_trade_timestamp = Some(trade_ts as u32);
        }
        s.ltp = price.to_string();
        s.cumulative_volume = Some(volume);
        s.connection_generation = generation;
        s.source_fresh = true;
        s
    }
    fn tick(ts: i64, price: i32, volume: u32, generation: u32) -> Snapshot {
        packet(ts, ts, price, volume, generation)
    }
    fn ms(s: i64) -> i64 {
        s * 1_000
    }
    fn ohlcv(c: &Candle) -> (f64, f64, f64, f64, u64) {
        (c.open, c.high, c.low, c.close, c.volume)
    }
    const START: i64 = 1_800_000_000 / 180 * 180;

    #[test]
    fn closes_on_the_clock_one_grace_after_the_boundary() {
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        a.observe(&tick(START + 30, 110, 1007, 1)).unwrap();
        a.observe(&tick(START + 120, 90, 1012, 1)).unwrap();
        assert!(a.close_due(ms(START + 180)).is_empty(), "boundary: grace not over");
        assert!(a.close_due(ms(START + 180) + 999).is_empty());
        let bars = a.close_due(ms(START + 180) + 1_000);
        assert_eq!(bars.len(), 1, "closes with no packet after the boundary");
        assert_eq!(ohlcv(&bars[0]), (100., 110., 90., 90., 12));
        assert_eq!(bars[0].timestamp, Utc.timestamp_opt(START, 0).single().unwrap().to_rfc3339());
        assert!(a.close_due(ms(START + 200)).is_empty(), "handed out once");
    }

    #[test]
    fn a_trade_goes_into_the_bar_of_its_trade_time() {
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        // sent at the boundary, but the trade happened at START+179: the old bar's close
        a.observe(&packet(START + 180, START + 179, 95, 1003, 1)).unwrap();
        // the first trade of the next bar
        a.observe(&packet(START + 180, START + 180, 97, 1005, 1)).unwrap();
        // another late-stamped trade of the old bar, still inside the grace window
        a.observe(&packet(START + 180, START + 179, 94, 1006, 1)).unwrap();
        let bars = a.close_due(ms(START + 181));
        assert_eq!(bars.len(), 1);
        assert_eq!(ohlcv(&bars[0]), (100., 100., 94., 94., 4));
        let next = a.close_due(ms(START + 361));
        assert_eq!(ohlcv(&next[0]), (97., 97., 97., 97., 2));
    }

    #[test]
    fn packets_without_a_new_trade_change_nothing() {
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        a.observe(&packet(START + 10, START + 10, 101, 1002, 1)).unwrap();
        // depth refresh: same trade time, same volume, even a stray price
        a.observe(&packet(START + 20, START + 10, 150, 1002, 1)).unwrap();
        let bars = a.close_due(ms(START + 181));
        assert_eq!(ohlcv(&bars[0]), (100., 101., 100., 101., 2));
    }

    #[test]
    fn a_trade_after_its_bar_closed_moves_its_volume_on_and_is_counted() {
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        assert_eq!(a.close_due(ms(START + 181)).len(), 1);
        a.observe(&packet(START + 182, START + 179, 99, 1004, 1)).unwrap();
        assert_eq!(a.late_trades(), 1);
        a.observe(&tick(START + 185, 102, 1005, 1)).unwrap();
        let bars = a.close_due(ms(START + 361));
        assert_eq!(ohlcv(&bars[0]), (102., 102., 102., 102., 5), "4 carried + 1 own");
    }

    #[test]
    fn a_period_without_trades_produces_no_bar_and_does_not_fail() {
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        assert_eq!(a.close_due(ms(START + 181)).len(), 1);
        // quiet tape: only depth refreshes until a trade at START+365
        a.observe(&packet(START + 300, START, 100, 1000, 1)).unwrap();
        assert!(a.close_due(ms(START + 361)).is_empty(), "no trade, no bar");
        a.observe(&tick(START + 365, 103, 1001, 1)).unwrap();
        let bars = a.close_due(ms(START + 541));
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].timestamp, Utc.timestamp_opt(START + 360, 0).single().unwrap().to_rfc3339());
    }

    #[test]
    fn five_minute_step_closes_only_on_five_minute_boundaries() {
        let start = 1_800_000_000i64.div_euclid(300) * 300;
        let mut a = Aggregator::with_step((start as u64) * 1_000_000_000, 300);
        a.observe(&tick(start, 100, 1000, 1)).unwrap();
        a.observe(&tick(start + 180, 120, 1010, 1)).unwrap();
        assert!(a.close_due(ms(start + 181)).is_empty(), "no 3-minute close");
        let bars = a.close_due(ms(start + 301));
        assert_eq!(ohlcv(&bars[0]), (100., 120., 100., 120., 10));
        assert_eq!(bars[0].timestamp, Utc.timestamp_opt(start, 0).single().unwrap().to_rfc3339());
    }

    #[test]
    fn skips_partial_start_and_refuses_reconnect_volume_reset_and_disorder() {
        let mut partial = Aggregator::new((START as u64) * 1_000_000_000);
        partial.observe(&tick(START + 20, 100, 1000, 1)).unwrap();
        partial.observe(&tick(START + 180, 101, 1001, 1)).unwrap();
        assert!(partial.close_due(ms(START + 181)).is_empty(), "partial first bar not handed out");
        assert_eq!(partial.close_due(ms(START + 361)).len(), 1);
        // the skipped partial candle is reported once for backfill
        assert_eq!(partial.take_gap(), Some((START, START + 180)));
        assert_eq!(partial.take_gap(), None);
        let mut gap = Aggregator::new(((START - 360) as u64) * 1_000_000_000);
        gap.observe(&tick(START, 100, 1000, 1)).unwrap();
        // first tick on the boundary: no partial, but two candles before it are missing
        assert_eq!(gap.take_gap(), Some((START - 360, START)));
        assert_eq!(gap.close_due(ms(START + 181)).len(), 1);
        let mut a = Aggregator::new((START as u64) * 1_000_000_000);
        a.observe(&tick(START, 100, 1000, 1)).unwrap();
        assert!(a.observe(&tick(START + 1, 101, 1001, 2)).is_err(), "new connection");
        let mut b = Aggregator::new((START as u64) * 1_000_000_000);
        b.observe(&tick(START, 100, 1000, 1)).unwrap();
        assert!(b.observe(&tick(START + 180, 101, 900, 1)).is_err(), "volume reset");
        let mut d = Aggregator::new((START as u64) * 1_000_000_000);
        d.observe(&tick(START, 100, 1000, 1)).unwrap();
        d.observe(&tick(START + 60, 110, 1001, 1)).unwrap();
        assert!(d.observe(&tick(START + 59, 109, 1002, 1)).is_err(), "packet time backwards");
        let mut e = Aggregator::new((START as u64) * 1_000_000_000);
        e.observe(&tick(START, 100, 1000, 1)).unwrap();
        assert!(e.observe(&packet(START + 10, START + 20, 101, 1001, 1)).is_err(), "trade after packet");
    }
}
