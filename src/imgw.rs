//! Networked IMGW endpoints: the app API token (scraped + cached), the HYBRID point
//! forecast, the reverse geocoder (point → administrative-area code), the warning
//! feeds, and the cached river-basin polygons. All fetches go through the client's
//! retry/backoff policy.

use crate::client::{Imgw, Service};
use crate::model::{AreaQuery, AreaResponse, ForecastQuery};
use anyhow::{anyhow, Result};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Last-known app token, used as a fallback when scraping fails.
pub const FALLBACK_TOKEN: &str = "p4DXKjsYadfBV21TYrDk";

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
pub fn token(client: &Imgw, cache_dir: &Path, refresh: bool) -> Result<String> {
    let cache: PathBuf = cache_dir.join(".imgw-token");
    if !refresh && fresh(&cache, TOKEN_TTL) {
        if let Ok(tok) = std::fs::read_to_string(&cache) {
            let tok = tok.trim().to_string();
            if !tok.is_empty() {
                return Ok(tok);
            }
        }
    }

    let meteo = &client.meteo;
    let index = client.get(&format!("{meteo}/"), 3)?;
    let main = find_between(&index, "main.", ".js")
        .map(|mid| format!("main.{mid}.js"))
        .ok_or_else(|| anyhow!("could not locate the JS bundle"))?;

    let bundle = client.get(&format!("{meteo}/{main}"), 3)?;
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
pub fn forecast(client: &Imgw, token: &str, lat: f64, lon: f64) -> Result<String> {
    let url = format!("{}/api/v1/forecast/fcapi", client.meteo);
    let query = ForecastQuery {
        token,
        lat,
        lon,
        mode: "hybrid",
    };
    client.get_query(&url, &query, 3)
}

/// The administrative-area code nearest a coordinate, via IMGW's reverse geocoder. None
/// on any failure (the meteo-warning box is simply skipped without an area to filter by).
pub fn area_code(client: &Imgw, token: &str, lat: f64, lon: f64) -> Option<String> {
    let url = format!("{}/api/v1/geo/search-reverse", client.meteo);
    let query = AreaQuery {
        token,
        lat,
        lon,
        range: 10,
    };
    let body = client.get_query(&url, &query, 3).ok()?;
    let parsed: AreaResponse = serde_json::from_str(&body).ok()?;
    nearest_area(&parsed)
}

/// Pick the area code of the nearest match (smallest distance) from a lookup response.
fn nearest_area(resp: &AreaResponse) -> Option<String> {
    resp.data
        .iter()
        .filter_map(|e| {
            let distance: f64 = e.distance.parse().ok()?;
            Some((distance, e.area.clone()))
        })
        .filter(|(_, area)| !area.is_empty())
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal))
        .map(|(_, area)| area)
}

/// Fetch a warning feed (retries past 404-ing nodes). Returns the body (may be `[]`
/// when there are no warnings), or None on total failure.
pub fn warning_feed(client: &Imgw, product: &str) -> Option<String> {
    let url = format!("{}/{product}", client.warnings);
    client.get(&url, 5).ok()
}

/// Ensure the river-basin polygons are cached and < 30 days old; return the file path.
pub fn ensure_basins(client: &Imgw, cache_dir: &Path) -> Option<PathBuf> {
    let path = cache_dir.join("imgw-zlew.json");
    let present = |p: &Path| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false);
    if fresh(&path, ZLEW_TTL) && present(&path) {
        return Some(path);
    }
    let url = format!("{}/dyn/data/zlew.json?v=1.38", client.meteo);
    if client.get_to_file(&url, &path, 3).is_ok() && present(&path) {
        return Some(path);
    }
    // fall back to a stale-but-present cache if the refresh failed
    if present(&path) {
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
        assert_eq!(
            find_between(html, "main.", ".js").unwrap(),
            "7fef0da67464963c"
        );
        let js = r#"...,apiToken:"p4DXKjsYadfBV21TYrDk",foo..."#;
        assert_eq!(
            find_between(js, "apiToken:\"", "\"").unwrap(),
            "p4DXKjsYadfBV21TYrDk"
        );
    }

    #[test]
    fn nearest_area_picks_smallest_distance() {
        let resp: AreaResponse = serde_json::from_str(
            r#"{"data":[
                {"teryt":"1206","dist":"0.90"},
                {"teryt":"1465","dist":"0.53"},
                {"teryt":"9999","dist":"1.20"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(nearest_area(&resp).unwrap(), "1465");
    }

    #[test]
    fn nearest_area_none_on_empty() {
        let resp: AreaResponse = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(nearest_area(&resp).is_none());
    }
}
