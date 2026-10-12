//! Bounded retry for classified read failures only. Mutations never retry.
use super::broker::{Broker, Snapshot};
use anyhow::{Result, anyhow};
use std::{future::Future, time::Duration};
#[derive(Debug)]
pub(crate) enum ReadFailure {
    Transient,
    SessionExpired,
    RateLimited(u64),
}
impl std::fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Transient => "Kite read temporarily unavailable",
            Self::SessionExpired => "Kite session rejected; renew the token in Redis",
            Self::RateLimited(_) => "Kite read rate limited; review cooldown",
        })
    }
}
impl std::error::Error for ReadFailure {}

/// kite-adapter 0.8.4: one line per failed Kite read, so a "temporarily unavailable" fault
/// shows what actually happened. A JSON `kite_read_failed` event goes to stdout (the run's
/// JSON log) and a readable line to stderr (the terminal). Path and cause only: never the
/// query string, headers or response body.
pub(crate) fn log_read_failure(path: &str, cause: &str, elapsed_ms: u128) {
    use std::io::Write as _;
    let ist = chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(19_800).expect("IST offset"));
    let event = serde_json::json!({
        "event": "kite_read_failed",
        "at_ist": ist.format("%H:%M:%S%.3f").to_string(),
        "path": path,
        "cause": cause,
        "elapsed_ms": elapsed_ms,
    });
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{event}");
        let _ = out.flush();
    }
    let _ = writeln!(
        std::io::stderr().lock(),
        "{} Kite read {path} failed: {cause} ({elapsed_ms} ms)",
        ist.format("%H:%M:%S")
    );
}

/// Full account snapshot (start-up, reports), with the bounded retry.
pub(crate) async fn snapshot(broker: &dyn Broker) -> Result<Snapshot> {
    read(|| broker.snapshot()).await
}

/// Up to 3 attempts, 12 s each; only `ReadFailure::Transient` (and a timeout) is retried,
/// after 250 ms and then 500 ms. Session, rate-limit and integrity errors return at once.
pub(crate) async fn read<T, F, Fut>(mut attempt_read: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    for attempt in 0..3 {
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(12), attempt_read())
            .await
            .unwrap_or_else(|_| {
                log_read_failure(
                    "(whole attempt)",
                    &format!("no answer within 12 s, attempt {}/3", attempt + 1),
                    started.elapsed().as_millis(),
                );
                Err(anyhow!(ReadFailure::Transient))
            });
        match result {
            Ok(value) => return Ok(value),
            Err(e)
                if matches!(e.downcast_ref::<ReadFailure>(), Some(ReadFailure::Transient))
                    && attempt < 2 =>
            {
                tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await
            }
            Err(e) => {
                if matches!(e.downcast_ref::<ReadFailure>(), Some(ReadFailure::Transient)) {
                    log_read_failure(
                        "(whole read)",
                        "giving up after 3 attempts",
                        started.elapsed().as_millis(),
                    );
                }
                return Err(e);
            }
        }
    }
    unreachable!()
}
