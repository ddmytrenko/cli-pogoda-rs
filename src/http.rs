//! Retry policy. Every remote call goes through `retry_with`, since IMGW's endpoints
//! sit behind a load-balancer pool where some nodes 404/422 a URL that others serve
//! fine. Between attempts it waits with exponential backoff plus jitter, so a flapping
//! node isn't hammered instantly.

use anyhow::{anyhow, Result};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Build the shared HTTP agent (gzip like `--compressed`, a sane timeout).
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent("imgw-rs")
        .build()
}

/// Backoff schedule between retry attempts: `base * 2^attempt`, capped at `max`, plus a
/// random 0..=`jitter`. All-zero disables waiting (used in tests to keep them instant).
#[derive(Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
    pub jitter: Duration,
}

impl Backoff {
    /// The default schedule used in normal operation.
    pub fn production() -> Self {
        Backoff {
            base: Duration::from_millis(200),
            max: Duration::from_secs(3),
            jitter: Duration::from_millis(250),
        }
    }

    /// No waiting at all — for tests, and for the zero-backoff default.
    pub fn none() -> Self {
        Backoff {
            base: Duration::ZERO,
            max: Duration::ZERO,
            jitter: Duration::ZERO,
        }
    }

    /// Delay to wait *before* the retry following the (0-based) `attempt`.
    pub fn delay(&self, attempt: u32) -> Duration {
        if self.base.is_zero() && self.jitter.is_zero() {
            return Duration::ZERO;
        }
        let base_ms = self.base.as_millis() as u64;
        let grown = base_ms.saturating_mul(1u64 << attempt.min(16));
        let capped = grown.min(self.max.as_millis() as u64);
        Duration::from_millis(capped) + self.jitter_amount()
    }

    /// A pseudo-random 0..=`jitter`, seeded from the wall clock (no extra deps).
    fn jitter_amount(&self) -> Duration {
        let j = self.jitter.as_millis() as u64;
        if j == 0 {
            return Duration::ZERO;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        Duration::from_millis(nanos % (j + 1))
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff::production()
    }
}

/// Run `f`, retrying while it returns `Err`, up to `max` attempts total (clamped to at
/// least 1), waiting per `backoff` between attempts. Returns the first `Ok`, else the
/// last `Err`.
pub fn retry_with<T, F>(max: u32, backoff: &Backoff, mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let max = max.max(1);
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..max {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if attempt + 1 < max {
                    let d = backoff.delay(attempt);
                    if !d.is_zero() {
                        std::thread::sleep(d);
                    }
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("no attempts made")))
}

/// Convenience: retry with no backoff.
pub fn retry<T, F>(max: u32, f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    retry_with(max, &Backoff::none(), f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn success_on_first_attempt_runs_once() {
        let calls = Cell::new(0);
        let r: Result<i32> = retry(3, || {
            calls.set(calls.get() + 1);
            Ok(1)
        });
        assert_eq!(r.unwrap(), 1);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retries_past_failures_then_succeeds() {
        let calls = Cell::new(0);
        let r: Result<i32> = retry(5, || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(anyhow!("boom"))
            } else {
                Ok(7)
            }
        });
        assert_eq!(r.unwrap(), 7);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let calls = Cell::new(0);
        let r: Result<i32> = retry(3, || {
            calls.set(calls.get() + 1);
            Err(anyhow!("always"))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn max_below_one_is_clamped_to_single_attempt() {
        let calls = Cell::new(0);
        let r: Result<i32> = retry(0, || {
            calls.set(calls.get() + 1);
            Err(anyhow!("always"))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let b = Backoff {
            base: Duration::from_millis(100),
            max: Duration::from_millis(500),
            jitter: Duration::ZERO,
        };
        assert_eq!(b.delay(0), Duration::from_millis(100)); // 100 * 2^0
        assert_eq!(b.delay(1), Duration::from_millis(200)); // 100 * 2^1
        assert_eq!(b.delay(2), Duration::from_millis(400)); // 100 * 2^2
        assert_eq!(b.delay(3), Duration::from_millis(500)); // 800 capped to 500
        assert_eq!(b.delay(10), Duration::from_millis(500)); // still capped
    }

    #[test]
    fn none_backoff_never_waits() {
        let b = Backoff::none();
        assert_eq!(b.delay(0), Duration::ZERO);
        assert_eq!(b.delay(5), Duration::ZERO);
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let b = Backoff {
            base: Duration::ZERO,
            max: Duration::ZERO,
            jitter: Duration::from_millis(50),
        };
        for _ in 0..100 {
            assert!(b.delay(0) <= Duration::from_millis(50));
        }
    }
}
