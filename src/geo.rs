//! Location resolution. IMGW has no geocoder, so a place string is turned into
//! lat/lon + display name + IANA timezone via external lookups: Open-Meteo forward
//! geocoding for names, Open-Meteo + BigDataCloud for bare coordinates.

use crate::client::Client;
use crate::model::{
    GeocodeQuery, GeocodeResults, OpenMeteoForecast, ReverseGeocode, ReverseGeocodeQuery,
    TimezoneQuery,
};
use anyhow::{anyhow, Result};

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

pub fn resolve(client: &Client, place: &str) -> Result<Location> {
    match parse_place(place) {
        Place::Coords(lat, lon) => resolve_coords(client, lat, lon),
        Place::Name(town) => resolve_name(client, &town),
    }
}

fn resolve_coords(client: &Client, lat: f64, lon: f64) -> Result<Location> {
    // timezone from Open-Meteo (auto), default UTC.
    let tz_query = TimezoneQuery {
        latitude: lat,
        longitude: lon,
        timezone: "auto",
        forecast_days: 1,
    };
    let tz = client
        .get_query(&client.endpoints.open_meteo, &tz_query, 3)
        .ok()
        .and_then(|b| serde_json::from_str::<OpenMeteoForecast>(&b).ok())
        .map(|r| r.timezone)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UTC".to_string());

    // display name from BigDataCloud reverse geocode, default "lat, lon".
    let name_query = ReverseGeocodeQuery {
        latitude: lat,
        longitude: lon,
        locality_language: "en",
    };
    let label = client
        .get_query(&client.endpoints.bigdatacloud, &name_query, 3)
        .ok()
        .and_then(|b| serde_json::from_str::<ReverseGeocode>(&b).ok())
        .and_then(|r| {
            [r.city, r.locality, r.principal_subdivision]
                .into_iter()
                .find(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("{lat}, {lon}"));

    Ok(Location {
        lat,
        lon,
        tz,
        label,
    })
}

fn resolve_name(client: &Client, town: &str) -> Result<Location> {
    let query = GeocodeQuery {
        name: town,
        count: 1,
        language: "en",
        format: "json",
    };
    let body = client
        .get_query(&client.endpoints.geocoding, &query, 3)
        .map_err(|_| anyhow!("geocoding request failed"))?;

    let parsed: GeocodeResults =
        serde_json::from_str(&body).map_err(|_| anyhow!("geocoding request failed"))?;
    let first = parsed
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("could not geocode \"{town}\""))?;

    let (lat, lon) = match (first.latitude, first.longitude) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(anyhow!("could not geocode \"{town}\"")),
    };
    let tz = if first.timezone.is_empty() {
        "UTC".to_string()
    } else {
        first.timezone
    };

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
        assert!(matches!(
            parse_place("  52.24 , 21.03 "),
            Place::Coords(_, _)
        ));
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
