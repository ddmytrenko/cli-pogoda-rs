//! Parse the IMGW HYBRID point-nowcast payload (`api/v1/forecast/fcapi`): pick the
//! current step (Data[0]), then aggregate precipitation over a de-duplicated step
//! series — every 10-minute step, plus hourly steps only beyond the last 10-minute
//! one, so the two cadences don't double-count rain. Pure: takes the JSON text,
//! returns a Forecast.

use crate::model::{ForecastResponse, ForecastStep};
use anyhow::{anyhow, Result};

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

/// Parse a numeric string field ("295.0"), or 0.0 when empty/unparseable.
fn num(s: &str) -> f64 {
    s.trim().parse().unwrap_or(0.0)
}

pub fn parse(json: &str) -> Result<Forecast> {
    let resp: ForecastResponse =
        serde_json::from_str(json).map_err(|_| anyhow!("bad forecast JSON"))?;
    let d = resp.data;

    if !d.valid {
        return Err(anyhow!("forecast not valid"));
    }
    if d.steps.is_empty() {
        return Err(anyhow!("no forecast steps"));
    }
    let c = &d.steps[0];

    // Build the de-duplicated step series (see module doc-comment above).
    let ten: Vec<&ForecastStep> = d
        .steps
        .iter()
        .filter(|s| s.kind == "Type_Ten_Minutes")
        .collect();
    let cutoff = ten.iter().map(|s| s.date.as_str()).max().unwrap_or("");
    let mut steps: Vec<&ForecastStep> = ten.clone();
    for s in &d.steps {
        if s.kind == "Type_Hour" && s.date.as_str() > cutoff {
            steps.push(s);
        }
    }
    if steps.is_empty() {
        steps = d.steps.iter().collect();
    }

    let temp = c
        .temperature
        .trim()
        .parse::<f64>()
        .map_err(|_| anyhow!("no temperature"))?
        - 273.15;
    let feels = c.chill.trim().parse::<f64>().unwrap_or(temp + 273.15) - 273.15;
    let pres = num(&c.pressure_msl) / 100.0;

    let prec_sum: f64 = steps.iter().map(|s| num(&s.precipitation)).sum();
    let rain_sum: f64 = steps.iter().map(|s| num(&s.rain)).sum();
    let snow_sum: f64 = steps.iter().map(|s| num(&s.snow)).sum();

    let hrs = match (steps.first(), steps.last()) {
        (Some(a), Some(b)) => match (
            chrono::DateTime::parse_from_rfc3339(&a.date),
            chrono::DateTime::parse_from_rfc3339(&b.date),
        ) {
            (Ok(ta), Ok(tb)) => (tb.timestamp() - ta.timestamp()) as f64 / 3600.0,
            _ => 0.0,
        },
        _ => 0.0,
    };

    Ok(Forecast {
        temp,
        feels,
        wspeed: c.wind_speed.clone(),
        wdir: c.wind_dir.clone(),
        gust: c.wind_gust.clone(),
        hum: c.humidity.clone(),
        pres,
        cloud: c.cloud.clone(),
        icon: c.icon.clone(),
        rain: num(&c.rain),
        snow: num(&c.snow),
        prec: num(&c.precipitation),
        sunrise: d.sun.sunrise.clone(),
        sunset: d.sun.sunset.clone(),
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
