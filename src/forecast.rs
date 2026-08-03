//! Parse the IMGW HYBRID point-nowcast payload (`api/v1/forecast/fcapi`): pick the
//! current step (Data[0]), then aggregate precipitation over a de-duplicated step
//! series — every 10-minute step, plus hourly steps only beyond the last 10-minute
//! one, so the two cadences don't double-count rain. Pure: takes the JSON text,
//! returns a Forecast.

use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Forecast {
    pub temp: f64,  // °C
    pub feels: f64, // °C
    pub wspeed: String,
    pub wdir: String,
    pub gust: String,
    pub hum: String,
    pub pres: f64, // hPa
    pub cloud: String,
    pub icon: String,
    pub rain: f64, // 10-min mm
    pub snow: f64,
    pub prec: f64,
    pub sunrise: String, // ISO-UTC
    pub sunset: String,
    pub prec_sum: f64, // mm over the step window
    pub rain_sum: f64,
    pub snow_sum: f64,
    pub hrs: f64, // span of the step window, hours
}

/// Field as a display string ("" when missing/null; numbers stringified).
fn vstr(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Field parsed to f64 (string or number), or None when missing/unparseable.
fn vnum_opt(v: &Value, key: &str) -> Option<f64> {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Like `vnum_opt` but 0.0 for missing/unparseable fields.
fn vnum(v: &Value, key: &str) -> f64 {
    vnum_opt(v, key).unwrap_or(0.0)
}

pub fn parse(json: &str) -> Result<Forecast> {
    let root: Value = serde_json::from_str(json).map_err(|_| anyhow!("bad forecast JSON"))?;
    let d = root.get("data").ok_or_else(|| anyhow!("no data"))?;

    if d.get("Valid").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("forecast not valid"));
    }
    let arr = d
        .get("Data")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| anyhow!("no forecast steps"))?;
    let c = &arr[0];

    // Build the de-duplicated step series (see module doc-comment above).
    let ten: Vec<&Value> = arr
        .iter()
        .filter(|s| s.get("Type").and_then(Value::as_str) == Some("Type_Ten_Minutes"))
        .collect();
    let last_ten: Option<&str> = ten
        .iter()
        .filter_map(|s| s.get("Date").and_then(Value::as_str))
        .max();
    let cutoff = last_ten.unwrap_or("");
    let mut steps: Vec<&Value> = ten.clone();
    for s in arr {
        if s.get("Type").and_then(Value::as_str) == Some("Type_Hour") {
            if let Some(date) = s.get("Date").and_then(Value::as_str) {
                if date > cutoff {
                    steps.push(s);
                }
            }
        }
    }
    if steps.is_empty() {
        steps = arr.iter().collect();
    }

    let temp = vnum_opt(c, "Temperature").ok_or_else(|| anyhow!("no temperature"))? - 273.15;
    let feels = vnum_opt(c, "Chill").unwrap_or(temp + 273.15) - 273.15;
    let pres = vnum_opt(c, "PressureMSL").unwrap_or(0.0) / 100.0;

    let prec_sum: f64 = steps.iter().map(|s| vnum(s, "Precipitation10m")).sum();
    let rain_sum: f64 = steps.iter().map(|s| vnum(s, "Rain10m")).sum();
    let snow_sum: f64 = steps.iter().map(|s| vnum(s, "Snow10m")).sum();

    let hrs = match (
        steps.first().and_then(|s| s.get("Date")).and_then(Value::as_str),
        steps.last().and_then(|s| s.get("Date")).and_then(Value::as_str),
    ) {
        (Some(a), Some(b)) => match (
            chrono::DateTime::parse_from_rfc3339(a),
            chrono::DateTime::parse_from_rfc3339(b),
        ) {
            (Ok(ta), Ok(tb)) => (tb.timestamp() - ta.timestamp()) as f64 / 3600.0,
            _ => 0.0,
        },
        _ => 0.0,
    };

    let sun = d.get("Sun");
    Ok(Forecast {
        temp,
        feels,
        wspeed: vstr(c, "Wind_Speed"),
        wdir: vstr(c, "Wind_Dir"),
        gust: vstr(c, "Wind_Gust"),
        hum: vstr(c, "Humidity"),
        pres,
        cloud: vstr(c, "Cloud"),
        icon: vstr(c, "Icon10"),
        rain: vnum(c, "Rain10m"),
        snow: vnum(c, "Snow10m"),
        prec: vnum(c, "Precipitation10m"),
        sunrise: sun.map(|s| vstr(s, "Sunrise")).unwrap_or_default(),
        sunset: sun.map(|s| vstr(s, "Sunset")).unwrap_or_default(),
        prec_sum,
        rain_sum,
        snow_sum,
        hrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal payload: one 10-min step (current) + two hourly steps, one of which
    // overlaps the 10-min window and must be dropped from the precip aggregation.
    const FIXTURE: &str = r#"{
      "data": {
        "Valid": true,
        "Sun": { "Sunrise": "2026-07-31T03:08:00Z", "Sunset": "2026-07-31T18:27:00Z" },
        "Data": [
          { "Type": "Type_Ten_Minutes", "Date": "2026-07-31T10:00:00Z",
            "Temperature": "300.0", "Chill": "301.0", "Wind_Speed": "3.1",
            "Wind_Dir": "134", "Wind_Gust": "8.4", "Humidity": "37",
            "PressureMSL": "101900", "Cloud": "0", "Icon10": "n0z00d",
            "Rain10m": "0.5", "Snow10m": "0.0", "Precipitation10m": "0.5" },
          { "Type": "Type_Hour", "Date": "2026-07-31T10:00:00Z",
            "Rain10m": "9.9", "Snow10m": "0.0", "Precipitation10m": "9.9" },
          { "Type": "Type_Hour", "Date": "2026-07-31T11:00:00Z",
            "Rain10m": "1.0", "Snow10m": "0.0", "Precipitation10m": "1.0" }
        ]
      }
    }"#;

    #[test]
    fn converts_units_and_fields() {
        let f = parse(FIXTURE).unwrap();
        assert!((f.temp - 26.85).abs() < 1e-6); // 300 - 273.15
        assert!((f.feels - 27.85).abs() < 1e-6);
        assert!((f.pres - 1019.0).abs() < 1e-6); // 101900 / 100
        assert_eq!(f.wspeed, "3.1");
        assert_eq!(f.wdir, "134");
        assert_eq!(f.icon, "n0z00d");
    }

    #[test]
    fn precip_sum_drops_overlapping_hourly_step() {
        let f = parse(FIXTURE).unwrap();
        // 0.5 (ten) + 1.0 (hour beyond last ten) — the 9.9 overlapping hour is excluded.
        assert!((f.prec_sum - 1.5).abs() < 1e-6);
        assert!((f.rain_sum - 1.5).abs() < 1e-6);
    }

    #[test]
    fn step_window_span_in_hours() {
        let f = parse(FIXTURE).unwrap();
        // first step 10:00, last step 11:00 -> 1h
        assert!((f.hrs - 1.0).abs() < 1e-6);
    }

    #[test]
    fn invalid_payload_is_rejected() {
        assert!(parse(r#"{"data":{"Valid":false,"Data":[]}}"#).is_err());
        assert!(parse(r#"{"data":{"Valid":true,"Data":[]}}"#).is_err());
    }
}
