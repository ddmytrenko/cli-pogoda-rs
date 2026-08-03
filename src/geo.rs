//! Location resolution. IMGW has no geocoder, so a place string is turned into
//! lat/lon + display name + IANA timezone via external lookups: Open-Meteo forward
//! geocoding for names, Open-Meteo + BigDataCloud for bare coordinates.

use crate::http;
use anyhow::{anyhow, Result};
use serde_json::Value;

pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
    pub label: String,
}

/// Parse a place string into either coordinates or a town name. A string of exactly
/// two comma-separated numbers ("52.24,21.03") is coordinates; anything else is a
/// name and we keep only the part before the first comma (dropping any ",CC").
pub enum Place {
    Coords(f64, f64),
    Name(String),
}

pub fn parse_place(place: &str) -> Place {
    let parts: Vec<&str> = place.split(',').map(|s| s.trim()).collect();
    if parts.len() == 2 {
        if let (Ok(lat), Ok(lon)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
            return Place::Coords(lat, lon);
        }
    }
    Place::Name(parts[0].to_string())
}

pub fn resolve(agent: &ureq::Agent, place: &str) -> Result<Location> {
    match parse_place(place) {
        Place::Coords(lat, lon) => resolve_coords(agent, lat, lon),
        Place::Name(town) => resolve_name(agent, &town),
    }
}

fn resolve_coords(agent: &ureq::Agent, lat: f64, lon: f64) -> Result<Location> {
    let (lat_s, lon_s) = (lat.to_string(), lon.to_string());

    // timezone from Open-Meteo (auto), default UTC.
    let tz = http::get_text(
        agent,
        "https://api.open-meteo.com/v1/forecast",
        &[
            ("latitude", lat_s.as_str()),
            ("longitude", lon_s.as_str()),
            ("timezone", "auto"),
            ("forecast_days", "1"),
        ],
        3,
    )
    .ok()
    .and_then(|b| serde_json::from_str::<Value>(&b).ok())
    .and_then(|v| v.get("timezone").and_then(Value::as_str).map(String::from))
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "UTC".to_string());

    // display name from BigDataCloud reverse geocode, default "lat, lon".
    let url = format!(
        "https://api.bigdatacloud.net/data/reverse-geocode-client?latitude={lat}&longitude={lon}&localityLanguage=en"
    );
    let label = http::get_text(agent, &url, &[], 3)
        .ok()
        .and_then(|b| serde_json::from_str::<Value>(&b).ok())
        .and_then(|v| {
            for k in ["city", "locality", "principalSubdivision"] {
                if let Some(s) = v.get(k).and_then(Value::as_str) {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
            None
        })
        .unwrap_or_else(|| format!("{lat}, {lon}"));

    Ok(Location { lat, lon, tz, label })
}

fn resolve_name(agent: &ureq::Agent, town: &str) -> Result<Location> {
    let body = http::get_text(
        agent,
        "https://geocoding-api.open-meteo.com/v1/search",
        &[
            ("name", town),
            ("count", "1"),
            ("language", "en"),
            ("format", "json"),
        ],
        3,
    )
    .map_err(|_| anyhow!("geocoding request failed"))?;

    let v: Value = serde_json::from_str(&body).map_err(|_| anyhow!("geocoding request failed"))?;
    let first = v
        .get("results")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("could not geocode \"{town}\""))?;

    let lat = first.get("latitude").and_then(Value::as_f64);
    let lon = first.get("longitude").and_then(Value::as_f64);
    let (lat, lon) = match (lat, lon) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(anyhow!("could not geocode \"{town}\"")),
    };
    let tz = first
        .get("timezone")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("UTC")
        .to_string();

    Ok(Location {
        lat,
        lon,
        tz,
        label: town.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords_are_recognised() {
        match parse_place("52.24,21.03") {
            Place::Coords(a, b) => {
                assert!((a - 52.24).abs() < 1e-9);
                assert!((b - 21.03).abs() < 1e-9);
            }
            _ => panic!("expected coords"),
        }
    }

    #[test]
    fn coords_tolerate_whitespace() {
        matches!(parse_place("  52.24 , 21.03 "), Place::Coords(_, _));
    }

    #[test]
    fn name_drops_country_suffix() {
        match parse_place("Warsaw,PL") {
            Place::Name(n) => assert_eq!(n, "Warsaw"),
            _ => panic!("expected name"),
        }
    }

    #[test]
    fn bare_name_is_a_name() {
        match parse_place("Krzeszowice") {
            Place::Name(n) => assert_eq!(n, "Krzeszowice"),
            _ => panic!("expected name"),
        }
    }
}
