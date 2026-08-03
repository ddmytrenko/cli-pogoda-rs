//! Thin HTTP layer: a shared agent plus `retry`. Every remote call goes through
//! `retry`, since IMGW's endpoints sit behind a load-balancer pool where some nodes
//! 404/422 a URL that others serve fine.

use anyhow::{anyhow, Result};
use std::time::Duration;

/// Build the shared HTTP agent (gzip like curl's `--compressed`, a sane timeout).
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent("imgw-rs")
        .build()
}

/// Run `f`, retrying while it returns `Err`, up to `max` attempts total (clamped to
/// at least 1). Returns the first `Ok`, otherwise the last `Err`.
pub fn retry<T, F>(max: u32, mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let max = max.max(1);
    let mut last: Option<anyhow::Error> = None;
    for _ in 0..max {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("no attempts made")))
}

/// GET a URL with query params, retrying, returning the body as a string.
pub fn get_text(agent: &ureq::Agent, url: &str, params: &[(&str, &str)], tries: u32) -> Result<String> {
    retry(tries, || {
        let mut req = agent.get(url);
        for (k, v) in params {
            req = req.query(k, v);
        }
        Ok(req.call()?.into_string()?)
    })
}

/// GET a URL (retrying) and stream the body into `path`. Used for large payloads.
pub fn get_to_file(agent: &ureq::Agent, url: &str, path: &std::path::Path, tries: u32) -> Result<()> {
    let body = retry(tries, || {
        let resp = agent.get(url).call()?;
        let mut buf = Vec::new();
        std::io::copy(&mut resp.into_reader(), &mut buf)?;
        Ok(buf)
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, body)?;
    Ok(())
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
}
