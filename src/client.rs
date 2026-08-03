//! The HTTP client: a shared agent, a retry/backoff policy, and the set of service
//! base URLs. Endpoints are injectable so tests can point every call at a local mock
//! server; the default is the real production set.

use crate::http::{self, Backoff};
use anyhow::Result;
use std::path::Path;

/// Base URLs for the external services the app talks to.
#[derive(Clone)]
pub struct Endpoints {
    /// meteo.imgw.pl base (token bundle, forecast, reverse geocode, basin polygons).
    pub meteo: String,
    /// danepubliczne warnings-products base (a `/<product>` is appended).
    pub danepubliczne: String,
    /// Open-Meteo point forecast (used only for a coordinate's timezone).
    pub open_meteo: String,
    /// Open-Meteo forward geocoding (name -> lat/lon + timezone).
    pub geocoding: String,
    /// BigDataCloud reverse geocoding (coordinate -> display name).
    pub bigdatacloud: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            meteo: "https://meteo.imgw.pl".into(),
            danepubliczne: "https://danepubliczne.imgw.pl/api/data".into(),
            open_meteo: "https://api.open-meteo.com/v1/forecast".into(),
            geocoding: "https://geocoding-api.open-meteo.com/v1/search".into(),
            bigdatacloud: "https://api.bigdatacloud.net/data/reverse-geocode-client".into(),
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
    backoff: Backoff,
    pub endpoints: Endpoints,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// Production client: real endpoints, exponential backoff with jitter.
    pub fn new() -> Self {
        Client {
            agent: http::agent(),
            backoff: Backoff::production(),
            endpoints: Endpoints::default(),
        }
    }

    /// Client with explicit endpoints and backoff (tests use this with `Backoff::none`).
    pub fn with(endpoints: Endpoints, backoff: Backoff) -> Self {
        Client {
            agent: http::agent(),
            backoff,
            endpoints,
        }
    }

    /// GET a URL with query params, retrying per the backoff policy; body as a string.
    pub fn get_text(&self, url: &str, params: &[(&str, &str)], tries: u32) -> Result<String> {
        http::retry_with(tries, &self.backoff, || {
            let mut req = self.agent.get(url);
            for (k, v) in params {
                req = req.query(k, v);
            }
            Ok(req.call()?.into_string()?)
        })
    }

    /// GET a URL (retrying) and stream the body into `path`. Used for large payloads.
    pub fn get_to_file(&self, url: &str, path: &Path, tries: u32) -> Result<()> {
        let body = http::retry_with(tries, &self.backoff, || {
            let resp = self.agent.get(url).call()?;
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
}
