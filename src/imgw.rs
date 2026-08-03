//! Networked IMGW endpoints: the app API token (scraped + cached), the HYBRID point
//! forecast, the reverse geocoder (for a point's powiat TERYT), the danepubliczne
//! warnings products, and the cached river-basin polygons (zlew.json). All fetches go
//! through http::retry.

use crate::http;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Last-known app token, used as a fallback when scraping fails.
pub const FALLBACK_TOKEN: &str = "p4DXKjsYadfBV21TYrDk";

const FORECAST_URL: &str = "https://meteo.imgw.pl/api/v1/forecast/fcapi";
const REVERSE_URL: &str = "https://meteo.imgw.pl/api/v1/geo/search-reverse";
const ZLEW_URL: &str = "https://meteo.imgw.pl/dyn/data/zlew.json?v=1.38";
const TOKEN_TTL: Duration = Duration::from_secs(12 * 3600);
const ZLEW_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

fn file_age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

fn fresh(path: &Path, ttl: Duration) -> bool {
    file_age(path).map(|age| age < ttl).unwrap_or(false)
}

/// Resolve the app's API token: scrape it from meteo.imgw.pl's JS bundle so it
/// survives IMGW rotating it. Cached for 12h; `refresh` forces a refetch.
pub fn token(agent: &ureq::Agent, cache_dir: &Path, refresh: bool) -> Result<String> {
    let cache: PathBuf = cache_dir.join(".imgw-token");
    if !refresh && fresh(&cache, TOKEN_TTL) {
        if let Ok(tok) = std::fs::read_to_string(&cache) {
            let tok = tok.trim().to_string();
            if !tok.is_empty() {
                return Ok(tok);
            }
        }
    }

    let index = http::get_text(agent, "https://meteo.imgw.pl/", &[], 3)?;
    let main = find_between(&index, "main.", ".js")
        .map(|mid| format!("main.{mid}.js"))
        .ok_or_else(|| anyhow!("could not locate the JS bundle"))?;

    let bundle = http::get_text(agent, &format!("https://meteo.imgw.pl/{main}"), &[], 3)?;
    let tok = find_between(&bundle, "apiToken:\"", "\"")
        .ok_or_else(|| anyhow!("could not extract apiToken"))?;

    std::fs::create_dir_all(cache_dir).ok();
    std::fs::write(&cache, &tok).ok();
    Ok(tok)
}

/// Find the substring strictly between the first `open` and the next `close` after it.
fn find_between(hay: &str, open: &str, close: &str) -> Option<String> {
    let start = hay.find(open)? + open.len();
    let rest = &hay[start..];
    let end = rest.find(close)?;
    let val = &rest[..end];
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

/// Fetch the HYBRID point forecast body for a coordinate, with the given token.
pub fn forecast(agent: &ureq::Agent, token: &str, lat: f64, lon: f64) -> Result<String> {
    let (lat, lon) = (lat.to_string(), lon.to_string());
    http::get_text(
        agent,
        FORECAST_URL,
        &[
            ("token", token),
            ("lat", lat.as_str()),
            ("lon", lon.as_str()),
            ("m", "hybrid"),
        ],
        3,
    )
}

/// The powiat TERYT code nearest a coordinate, via IMGW's reverse geocoder. None on
/// any failure (the meteo-warning box is simply skipped without a TERYT to filter by).
pub fn reverse_teryt(agent: &ureq::Agent, token: &str, lat: f64, lon: f64) -> Option<String> {
    let (lat, lon) = (lat.to_string(), lon.to_string());
    let body = http::get_text(
        agent,
        REVERSE_URL,
        &[
            ("token", token),
            ("lat", lat.as_str()),
            ("lon", lon.as_str()),
            ("range", "10"),
        ],
        3,
    )
    .ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    nearest_teryt(&v)
}

/// Pick the TERYT of the nearest match (smallest `dist`) from a reverse-geocode body.
fn nearest_teryt(v: &Value) -> Option<String> {
    let data = v.get("data").and_then(Value::as_array)?;
    data.iter()
        .filter_map(|e| {
            let dist: f64 = e.get("dist").and_then(Value::as_str)?.parse().ok()?;
            let teryt = e.get("teryt").and_then(Value::as_str)?.to_string();
            Some((dist, teryt))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, teryt)| teryt)
}

/// Fetch a danepubliczne warnings product (retries past 404-ing nodes). Returns the
/// body (may be `[]` when there are no warnings), or None on total failure.
pub fn danepubliczne(agent: &ureq::Agent, product: &str) -> Option<String> {
    let url = format!("https://danepubliczne.imgw.pl/api/data/{product}");
    http::get_text(agent, &url, &[], 5).ok()
}

/// Ensure zlew.json (river-basin polygons) is cached and < 30 days old; return its path.
pub fn ensure_zlew(agent: &ureq::Agent, cache_dir: &Path) -> Option<PathBuf> {
    let path = cache_dir.join("imgw-zlew.json");
    if fresh(&path, ZLEW_TTL) && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
        return Some(path);
    }
    if http::get_to_file(agent, ZLEW_URL, &path, 3).is_ok() {
        if std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            return Some(path);
        }
    }
    // fall back to a stale-but-present cache if the refresh failed
    if std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
        Some(path)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bundle_name_and_token() {
        let html = r#"<script src="main.7fef0da67464963c.js"></script>"#;
        assert_eq!(find_between(html, "main.", ".js").unwrap(), "7fef0da67464963c");
        let js = r#"...,apiToken:"p4DXKjsYadfBV21TYrDk",foo..."#;
        assert_eq!(find_between(js, "apiToken:\"", "\"").unwrap(), "p4DXKjsYadfBV21TYrDk");
    }

    #[test]
    fn nearest_teryt_picks_smallest_dist() {
        let v: Value = serde_json::from_str(
            r#"{"data":[
                {"teryt":"1206","dist":"0.90"},
                {"teryt":"1465","dist":"0.53"},
                {"teryt":"9999","dist":"1.20"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(nearest_teryt(&v).unwrap(), "1465");
    }

    #[test]
    fn nearest_teryt_none_on_empty() {
        let v: Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(nearest_teryt(&v).is_none());
    }
}
