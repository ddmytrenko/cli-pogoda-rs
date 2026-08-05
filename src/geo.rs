//! Location resolution. IMGW has no geocoder, so a place string is turned into
//! lat/lon + display name + IANA timezone via external lookups: Open-Meteo forward
//! geocoding for names, Open-Meteo + BigDataCloud for bare coordinates.

use crate::client::Client;
use crate::model::{
    GeocodeQuery, GeocodeResult, GeocodeResults, OpenMeteoForecast, ReverseGeocode,
    ReverseGeocodeQuery, TimezoneQuery,
};
use anyhow::{anyhow, Result};

/// How many forward-geocoding candidates to fetch so we can pick the one in the
/// requested country (Open-Meteo has no country filter param).
const GEOCODE_CANDIDATES: u32 = 10;

pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
    pub label: String,
}

/// Parse a place string into either coordinates or a town name with an optional country.
/// A string of exactly two comma-separated numbers ("52.24,21.03") is coordinates;
/// otherwise it's a `Name`, split into the town and an optional `,CC` country code
/// (e.g. "Warszawa,PL" → town "Warszawa", country "PL").
pub enum Place {
    Coords(f64, f64),
    Name {
        town: String,
        country: Option<String>,
    },
}

pub fn parse_place(place: &str) -> Place {
    let parts: Vec<&str> = place.split(',').map(|s| s.trim()).collect();
    if parts.len() == 2 {
        if let (Ok(lat), Ok(lon)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
            return Place::Coords(lat, lon);
        }
    }
    let town = parts[0].to_string();
    let country = parts
        .get(1)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    Place::Name { town, country }
}

pub fn resolve(client: &Client, place: &str) -> Result<Location> {
    match parse_place(place) {
        Place::Coords(lat, lon) => resolve_coords(client, lat, lon),
        Place::Name { town, country } => resolve_name(client, &town, country.as_deref()),
    }
}

/// Pick the geocoding result to use. With a `country` (ISO-3166 alpha-2, case-insensitive)
/// choose the first candidate in that country; without one, the first candidate. Returns
/// `None` if a country was given but no candidate matches it — better than silently
/// resolving to a same-named place on another continent.
fn select_result<'a>(
    results: &'a [GeocodeResult],
    country: Option<&str>,
) -> Option<&'a GeocodeResult> {
    match country {
        Some(cc) => results
            .iter()
            .find(|r| r.country_code.eq_ignore_ascii_case(cc)),
        None => results.first(),
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

fn resolve_name(client: &Client, town: &str, country: Option<&str>) -> Result<Location> {
    // Fetch several candidates (Open-Meteo has no country filter) so `select_result`
    // can pick the one in the requested country.
    let count = if country.is_some() {
        GEOCODE_CANDIDATES
    } else {
        1
    };
    let query = GeocodeQuery {
        name: town,
        count,
        // Match on Polish endonyms (IMGW is a Poland-only service). Open-Meteo indexes
        // per language, so "pl" finds "Warszawa"/"Łódź" that "en" misses; the name is
        // sent as typed (diacritics intact) — folding to ASCII would break "Łódź".
        language: "pl",
        format: "json",
    };
    let body = client
        .get_query(&client.endpoints.geocoding, &query, 3)
        .map_err(|_| anyhow!("geocoding request failed"))?;

    let parsed: GeocodeResults =
        serde_json::from_str(&body).map_err(|_| anyhow!("geocoding request failed"))?;

    let not_found = || match country {
        Some(cc) => anyhow!("could not geocode \"{town}\" in {cc}"),
        None => anyhow!("could not geocode \"{town}\""),
    };
    let hit = select_result(&parsed.results, country).ok_or_else(not_found)?;

    let (lat, lon) = match (hit.latitude, hit.longitude) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(not_found()),
    };
    let tz = if hit.timezone.is_empty() {
        "UTC".to_string()
    } else {
        hit.timezone.clone()
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
    fn name_keeps_country_suffix() {
        match parse_place("Warszawa,PL") {
            Place::Name { town, country } => {
                assert_eq!(town, "Warszawa");
                assert_eq!(country.as_deref(), Some("PL"));
            }
            _ => panic!("expected name"),
        }
    }

    #[test]
    fn bare_name_has_no_country() {
        match parse_place("Krzeszowice") {
            Place::Name { town, country } => {
                assert_eq!(town, "Krzeszowice");
                assert_eq!(country, None);
            }
            _ => panic!("expected name"),
        }
    }

    fn result(country_code: &str, tz: &str) -> GeocodeResult {
        GeocodeResult {
            latitude: Some(0.0),
            longitude: Some(0.0),
            timezone: tz.into(),
            country_code: country_code.into(),
        }
    }

    #[test]
    fn select_result_prefers_the_requested_country() {
        // "Warszawa" resolves to a US place first, then the Polish capital — with
        // country "PL" we must pick the Polish one, not the first (US) hit.
        let results = vec![
            result("US", "America/New_York"),
            result("PL", "Europe/Warsaw"),
        ];
        let hit = select_result(&results, Some("PL")).unwrap();
        assert_eq!(hit.timezone, "Europe/Warsaw");
        // case-insensitive
        assert_eq!(
            select_result(&results, Some("pl")).unwrap().timezone,
            "Europe/Warsaw"
        );
    }

    #[test]
    fn select_result_none_when_country_absent_from_candidates() {
        let results = vec![
            result("US", "America/New_York"),
            result("DE", "Europe/Berlin"),
        ];
        assert!(select_result(&results, Some("PL")).is_none());
    }

    #[test]
    fn select_result_without_country_takes_first() {
        let results = vec![
            result("US", "America/New_York"),
            result("PL", "Europe/Warsaw"),
        ];
        assert_eq!(select_result(&results, None).unwrap().country_code, "US");
        assert!(select_result(&[], None).is_none());
    }
}
