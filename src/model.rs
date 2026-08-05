//! Typed models for the external JSON payloads the app consumes, deserialized with
//! serde. **Identifiers are English**; the (Polish / PascalCase / camelCase) wire names
//! are mapped with `#[serde(rename)]`. Every struct is `#[serde(default)]` so missing
//! fields fall back to their type default and unknown fields are ignored. This is the
//! single place that knows the shape of each upstream response — no ad-hoc string-keyed
//! lookups elsewhere.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------------
// Request query params (serialized to the URL query string via serde_urlencoded).
// ---------------------------------------------------------------------------------

/// Query for the IMGW HYBRID point forecast (`/api/v1/forecast/fcapi`).
#[derive(Debug, Serialize)]
pub struct ForecastQuery<'a> {
    pub token: &'a str,
    pub lat: f64,
    pub lon: f64,
    #[serde(rename = "m")]
    pub mode: &'a str,
}

/// Query for the IMGW reverse geocoder (`/api/v1/geo/search-reverse`) → area code.
#[derive(Debug, Serialize)]
pub struct AreaQuery<'a> {
    pub token: &'a str,
    pub lat: f64,
    pub lon: f64,
    pub range: u32,
}

/// Query for Open-Meteo's forecast endpoint, used only for a coordinate's timezone.
#[derive(Debug, Serialize)]
pub struct TimezoneQuery {
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: &'static str,
    pub forecast_days: u32,
}

/// Query for Open-Meteo forward geocoding (name → lat/lon + timezone).
#[derive(Debug, Serialize)]
pub struct GeocodeQuery<'a> {
    pub name: &'a str,
    pub count: u32,
    pub language: &'static str,
    pub format: &'static str,
}

/// Query for BigDataCloud reverse geocoding (coordinate → display name).
#[derive(Debug, Serialize)]
pub struct ReverseGeocodeQuery {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(rename = "localityLanguage")]
    pub locality_language: &'static str,
}

// ---------------------------------------------------------------------------------
// Response bodies.
// ---------------------------------------------------------------------------------

/// Top level of the HYBRID point forecast (`/api/v1/forecast/fcapi`).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ForecastResponse {
    pub data: ForecastData,
}

/// The forecast payload: validity flag, the step series, and sun times.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ForecastData {
    #[serde(rename = "Valid")]
    pub valid: bool,
    #[serde(rename = "Data")]
    pub steps: Vec<ForecastStep>,
    #[serde(rename = "Sun")]
    pub sun: Sun,
}

/// Sunrise/sunset for the forecast day (ISO-UTC strings).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Sun {
    #[serde(rename = "Sunrise")]
    pub sunrise: String,
    #[serde(rename = "Sunset")]
    pub sunset: String,
}

/// One forecast step. Numeric fields arrive as strings (e.g. "295.0"); callers parse
/// the ones they need. `kind` is "Type_Ten_Minutes" or "Type_Hour".
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ForecastStep {
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(rename = "Date")]
    pub date: String, // ISO-UTC
    #[serde(rename = "Temperature")]
    pub temperature: String, // Kelvin
    #[serde(rename = "Chill")]
    pub chill: String, // Kelvin
    #[serde(rename = "Wind_Speed")]
    pub wind_speed: String,
    #[serde(rename = "Wind_Dir")]
    pub wind_dir: String,
    #[serde(rename = "Wind_Gust")]
    pub wind_gust: String,
    #[serde(rename = "Humidity")]
    pub humidity: String,
    #[serde(rename = "PressureMSL")]
    pub pressure_msl: String, // Pa
    #[serde(rename = "Cloud")]
    pub cloud: String,
    #[serde(rename = "Icon10")]
    pub icon: String,
    #[serde(rename = "Rain10m")]
    pub rain: String, // mm
    #[serde(rename = "Snow10m")]
    pub snow: String, // mm
    #[serde(rename = "Precipitation10m")]
    pub precipitation: String, // mm
}

/// One entry of the `warningsmeteo` feed (a nationwide meteorological warning).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct MeteoWarning {
    #[serde(rename = "nazwa_zdarzenia")]
    pub event: String,
    #[serde(rename = "stopien")]
    pub level: String, // severity, "1".."3"
    #[serde(rename = "prawdopodobienstwo")]
    pub probability: String, // e.g. "90"
    #[serde(rename = "obowiazuje_od")]
    pub valid_from: String,
    #[serde(rename = "obowiazuje_do")]
    pub valid_until: String,
    #[serde(rename = "tresc")]
    pub description: String,
    #[serde(rename = "komentarz")]
    pub remarks: String,
    #[serde(rename = "teryt")]
    pub areas: Vec<String>, // administrative-area codes this warning covers
}

/// One entry of the `warningshydro` feed (a hydrological warning; level -1 = drought).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct HydroWarning {
    #[serde(rename = "stopień")]
    pub level: String, // severity, "1".."3", or "-1" for drought
    #[serde(rename = "prawdopodobienstwo")]
    pub probability: String,
    #[serde(rename = "data_od")]
    pub valid_from: String,
    #[serde(rename = "data_do")]
    pub valid_until: String,
    #[serde(rename = "zdarzenie")]
    pub event: String,
    #[serde(rename = "przebieg")]
    pub description: String,
    #[serde(rename = "komentarz")]
    pub remarks: String,
    #[serde(rename = "obszary")]
    pub areas: Vec<HydroArea>,
}

/// An affected area of a hydrological warning.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct HydroArea {
    #[serde(rename = "kod_zlewni")]
    pub basin_codes: Vec<String>,
}

/// Open-Meteo forecast response — only the timezone is used (for bare coordinates).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct OpenMeteoForecast {
    pub timezone: String,
}

/// Open-Meteo forward-geocoding response.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct GeocodeResults {
    pub results: Vec<GeocodeResult>,
}

/// One forward-geocoding match.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct GeocodeResult {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: String,
    #[serde(rename = "country_code")]
    pub country_code: String, // ISO-3166 alpha-2, e.g. "PL"
}

/// BigDataCloud reverse-geocoding response (coordinate → display name).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ReverseGeocode {
    pub city: String,
    pub locality: String,
    #[serde(rename = "principalSubdivision")]
    pub principal_subdivision: String,
}

/// IMGW reverse-geocoder response (`/api/v1/geo/search-reverse`).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AreaResponse {
    pub data: Vec<AreaMatch>,
}

/// One reverse-geocoder match: its administrative-area code and distance from the point.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AreaMatch {
    #[serde(rename = "teryt")]
    pub area: String,
    #[serde(rename = "dist")]
    pub distance: String,
}

/// River-basin polygons — a GeoJSON FeatureCollection.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BasinCollection {
    pub features: Vec<BasinFeature>,
}

/// One basin feature.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BasinFeature {
    pub geometry: BasinGeometry,
    pub properties: BasinProperties,
}

/// A basin's geometry. `coordinates` stays untyped: GeoJSON nests them differently for
/// Polygon vs MultiPolygon, and they're bare number arrays (no keys to name).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BasinGeometry {
    #[serde(rename = "type")]
    pub kind: String,
    pub coordinates: Value,
}

/// A basin's properties: its code and human name.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BasinProperties {
    #[serde(rename = "KOD")]
    pub code: String,
    #[serde(rename = "NAZWA")]
    pub name: String,
}
