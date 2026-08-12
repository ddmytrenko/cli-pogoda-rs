//! HTTP clients, one per external service. A low-level `Http` transport (a ureq agent
//! plus a retry/backoff policy) is shared in spirit by three service clients:
//!
//! * `Imgw` — meteo.imgw.pl (token bundle, HYBRID forecast, reverse geocoder, basin
//!   polygons) and danepubliczne.imgw.pl (the warning feeds);
//! * `Geocoding` — Open-Meteo forward geocoding (name → lat/lon) and a point forecast
//!   used only for a coordinate's timezone;
//! * `BigDataCloud` — reverse geocoding (coordinate → display name).
//!
//! Base URLs are injectable via `Endpoints` so tests can point every call at a local
//! mock server; the default is the real production set.

use crate::http::{self, Backoff};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

/// Low-level HTTP transport: a ureq agent and a retry/backoff policy. Service-agnostic;
/// the service clients below each own one and add their base URLs.
pub struct Http {
    agent: ureq::Agent,
    backoff: Backoff,
}

impl Http {
    /// A transport with the given backoff policy (tests pass `Backoff::none`).
    pub fn new(backoff: Backoff) -> Self {
        Http {
            agent: http::agent(),
            backoff,
        }
    }

    /// GET a URL (no query), retrying per the backoff policy; body as a string.
    pub fn get(&self, url: &str, tries: u32) -> Result<String> {
        http::retry_with(tries, &self.backoff, || {
            Ok(self.agent.get(url).call()?.into_string()?)
        })
    }

    /// GET a URL with a typed query (serialized to the query string), retrying; body as
    /// a string.
    pub fn get_query<Q: Serialize>(&self, url: &str, query: &Q, tries: u32) -> Result<String> {
        let qs = serde_urlencoded::to_string(query)?;
        let full = if qs.is_empty() {
            url.to_string()
        } else {
            format!("{url}?{qs}")
        };
        self.get(&full, tries)
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

/// The GET helpers, shared by every service client: expose the transport and the
/// `get`/`get_query`/`get_to_file` methods come for free.
pub trait Service {
    fn http(&self) -> &Http;

    fn get(&self, url: &str, tries: u32) -> Result<String> {
        self.http().get(url, tries)
    }
    fn get_query<Q: Serialize>(&self, url: &str, query: &Q, tries: u32) -> Result<String> {
        self.http().get_query(url, query, tries)
    }
    fn get_to_file(&self, url: &str, path: &Path, tries: u32) -> Result<()> {
        self.http().get_to_file(url, path, tries)
    }
}

/// IMGW-PIB. `meteo` is the meteo.imgw.pl base (token bundle, forecast, reverse
/// geocoder, basin polygons); `warnings` is the public-data feed base (a `/<product>`
/// is appended).
pub struct Imgw {
    http: Http,
    pub meteo: String,
    pub warnings: String,
}
impl Service for Imgw {
    fn http(&self) -> &Http {
        &self.http
    }
}

/// Open-Meteo. `search` is forward geocoding (name → lat/lon + timezone); `timezone` is
/// the point-forecast endpoint queried only for a bare coordinate's timezone.
pub struct Geocoding {
    http: Http,
    pub search: String,
    pub timezone: String,
}
impl Service for Geocoding {
    fn http(&self) -> &Http {
        &self.http
    }
}

/// BigDataCloud. `reverse` turns a coordinate into a display name.
pub struct BigDataCloud {
    http: Http,
    pub reverse: String,
}
impl Service for BigDataCloud {
    fn http(&self) -> &Http {
        &self.http
    }
}

/// Base URLs for every external service, injectable so tests can point them at a mock.
#[derive(Clone)]
pub struct Endpoints {
    /// meteo.imgw.pl base (token bundle, forecast, reverse geocode, basin polygons).
    pub meteo: String,
    /// Public-data warning-feeds base (a `/<product>` is appended).
    pub warnings: String,
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
            warnings: "https://danepubliczne.imgw.pl/api/data".into(),
            open_meteo: "https://api.open-meteo.com/v1/forecast".into(),
            geocoding: "https://geocoding-api.open-meteo.com/v1/search".into(),
            bigdatacloud: "https://api.bigdatacloud.net/data/reverse-geocode-client".into(),
        }
    }
}

/// The three service clients. Pass `&Clients` around; each fetch borrows the specific
/// client it needs. `thread::scope` in `run_place` shares `&Clients` across threads.
pub struct Clients {
    pub imgw: Imgw,
    pub geocoding: Geocoding,
    pub bigdatacloud: BigDataCloud,
}

impl Default for Clients {
    fn default() -> Self {
        Self::new()
    }
}

impl Clients {
    /// Production clients: real endpoints, exponential backoff with jitter.
    pub fn new() -> Self {
        Self::with(Endpoints::default(), Backoff::production())
    }

    /// Clients with explicit endpoints and backoff (tests use this with `Backoff::none`).
    /// Each service gets its own transport, so they never share connection state.
    pub fn with(endpoints: Endpoints, backoff: Backoff) -> Self {
        Clients {
            imgw: Imgw {
                http: Http::new(backoff),
                meteo: endpoints.meteo,
                warnings: endpoints.warnings,
            },
            geocoding: Geocoding {
                http: Http::new(backoff),
                search: endpoints.geocoding,
                timezone: endpoints.open_meteo,
            },
            bigdatacloud: BigDataCloud {
                http: Http::new(backoff),
                reverse: endpoints.bigdatacloud,
            },
        }
    }
}
