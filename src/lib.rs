//! `imgw` — a terminal point-weather nowcast from IMGW-PIB, the Polish
//! meteorological service. Resolves a location to coordinates, fetches the HYBRID
//! point forecast, and prints it with any active meteorological and hydrological
//! (drought) warnings for that exact point.

pub mod client;
pub mod config;
pub mod forecast;
pub mod geo;
pub mod http;
pub mod imgw;
pub mod ui;
pub mod warnings;
pub mod weather;

use client::Client;
use config::Config;
use std::io::Write;
use std::path::Path;
use ui::Colors;

/// Entry point used by the binary: parse args + config, then run and print to stdout.
pub fn run() -> i32 {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return 1;
        }
    };

    let cfg = Config::load();

    if args.help {
        print_help(&cfg);
        return 0;
    }

    // -p/--place wins, else a bare positional, else the configured default.
    let place = args
        .place
        .or(args.positional)
        .or_else(|| cfg.get("weather_place").map(String::from));
    let place = match place {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            eprintln!("Error: no location specified and `weather_place` is not set");
            eprintln!("Usage: imgw [-p|--place \"City,CC\"|\"lat,lon\"] [location]");
            eprintln!("Example: imgw -p \"Warsaw,PL\"");
            return 1;
        }
    };

    let client = Client::new();
    let cache = config::cache_dir();
    let colors = Colors::detect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    run_place(&client, &place, cache.as_deref(), &colors, &mut out)
}

/// Fetch and render the forecast (with warnings) for `place`, writing to `out`. Returns
/// a process exit code; user-facing errors go to stderr. This is the testable core:
/// point `client`'s endpoints at a mock server and capture `out`.
pub fn run_place(
    client: &Client,
    place: &str,
    cache: Option<&Path>,
    colors: &Colors,
    out: &mut impl Write,
) -> i32 {
    // 1. Resolve to lat/lon + display name + timezone.
    let loc = match geo::resolve(client, place) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("imgw: {e}");
            return 1;
        }
    };

    // 2. Point forecast. Resolve the token; on failure refresh it once.
    let mut token = cache
        .and_then(|d| imgw::token(client, d, false).ok())
        .unwrap_or_else(|| imgw::FALLBACK_TOKEN.to_string());

    let mut body = imgw::forecast(client, &token, loc.lat, loc.lon)
        .ok()
        .filter(|b| !b.trim().is_empty());
    if body.is_none() {
        if let Some(d) = cache {
            if let Ok(t) = imgw::token(client, d, true) {
                token = t;
                body = imgw::forecast(client, &token, loc.lat, loc.lon)
                    .ok()
                    .filter(|b| !b.trim().is_empty());
            }
        }
    }
    let body = match body {
        Some(b) => b,
        None => {
            eprintln!("imgw: IMGW forecast request failed");
            return 1;
        }
    };

    let fc = match forecast::parse(&body) {
        Ok(f) => f,
        Err(_) => {
            eprintln!(
                "imgw: no forecast data for \"{}\" ({}, {})",
                loc.label, loc.lat, loc.lon
            );
            return 1;
        }
    };

    // 3. Warning boxes, above the forecast: meteo (loud, red) then drought (quiet,
    // grey), each filtered to this exact point.
    print_warnings(client, colors, &token, &loc, cache, out);

    // 4. The forecast itself.
    print_forecast(&fc, &loc, out);

    0
}

/// Write any warning boxes that apply to this point.
fn print_warnings(
    client: &Client,
    colors: &Colors,
    token: &str,
    loc: &geo::Location,
    cache: Option<&Path>,
    out: &mut impl Write,
) {
    // "Today" for relative timestamp formatting, in the warnings' timezone (Poland).
    let today = chrono::Utc::now()
        .with_timezone(&chrono_tz::Europe::Warsaw)
        .date_naive();

    // Meteo warnings, filtered to this point's powiat TERYT, one box per severity
    // level (3=red, 2=orange, 1=yellow), highest first.
    if let Some(teryt) = imgw::reverse_teryt(client, token, loc.lat, loc.lon) {
        if let Some(raw) = imgw::danepubliczne(client, "warningsmeteo") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                let ws = warnings::meteo_warnings(&json, &teryt, today);
                for (level, lines) in warnings::boxes_by_level(&ws) {
                    for l in ui::warn_box(colors.level(level), &colors.reset, "WARNING!", &lines) {
                        let _ = writeln!(out, "{l}");
                    }
                }
            }
        }
    }

    // Hydrological warnings, filtered to this point's river basin: regular warnings
    // (levels 1/2/3) as coloured boxes, then the drought (susza, level -1) as a grey
    // notice.
    if let Some(cache) = cache {
        if let Some(zlew) = imgw::ensure_zlew(client, cache) {
            if let Some(basin) = warnings::find_basin(&zlew, loc.lat, loc.lon) {
                if let Some(raw) = imgw::danepubliczne(client, "warningshydro") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                        let ws = warnings::hydro_warnings(&json, &basin.kod, today);
                        for (level, lines) in warnings::boxes_by_level(&ws) {
                            for l in
                                ui::warn_box(colors.level(level), &colors.reset, "WARNING!", &lines)
                            {
                                let _ = writeln!(out, "{l}");
                            }
                        }
                        if warnings::drought_hits_basin(&json, &basin.kod) {
                            let line = format!(
                                "Susza hydrologiczna (hydrological drought) — {} basin",
                                basin.nazwa
                            );
                            for l in ui::warn_box(&colors.grey, &colors.reset, "NOTICE", &[line]) {
                                let _ = writeln!(out, "{l}");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Write the forecast block: banner, precip, wind, pressure, humidity, cloud, sun.
fn print_forecast(fc: &forecast::Forecast, loc: &geo::Location, out: &mut impl Write) {
    let cond = weather::condition(&fc.icon, fc.rain, fc.snow, fc.prec);
    let cond_suffix = if cond.is_empty() {
        String::new()
    } else {
        format!(", {cond}")
    };
    let _ = writeln!(
        out,
        " Weather in {}: {:.1} °C{}  (IMGW HYBRID, feels {:.1} °C)",
        loc.label, fc.temp, cond_suffix, fc.feels
    );

    let h = fc.hrs.round() as i64;
    if fc.prec > 0.0 || fc.prec_sum > 0.0 {
        let ptype = if fc.rain_sum > 0.0 && fc.snow_sum > 0.0 {
            "Rain & snow"
        } else if fc.snow_sum > 0.0 {
            "Snow"
        } else if fc.rain_sum > 0.0 {
            "Rain"
        } else {
            "Precipitation"
        };
        let _ = writeln!(
            out,
            "   {}: {:.1} mm now, {:.1} mm over next {}h",
            ptype, fc.prec, fc.prec_sum, h
        );
    } else {
        let _ = writeln!(out, "   Precipitation: none (dry next {h}h)");
    }

    if !fc.wspeed.is_empty() {
        let card = weather::cardinal(&fc.wdir);
        let _ = write!(out, "   Wind: {} m/s {} ({}°)", fc.wspeed, card, fc.wdir);
        if !fc.gust.is_empty() {
            let _ = write!(out, ", gust {} m/s", fc.gust);
        }
        let _ = writeln!(out);
    }
    if fc.pres > 0.0 {
        let _ = writeln!(out, "   Pressure: {:.0} hPa", fc.pres);
    }
    if !fc.hum.is_empty() {
        let _ = writeln!(out, "   Humidity: {}%", fc.hum);
    }
    if !fc.cloud.is_empty() {
        let _ = writeln!(out, "   Cloud: {}%", fc.cloud);
    }
    if let Some((sr, ss, daysec)) = sun_times(&fc.sunrise, &fc.sunset, &loc.tz) {
        let _ = writeln!(
            out,
            "   Sunrise: {}   Sunset: {}   (day {}h {:02}m)",
            sr,
            ss,
            daysec / 3600,
            (daysec % 3600) / 60
        );
    }
}

/// Convert ISO-UTC sunrise/sunset to local HH:MM in `tz`, plus the day length in
/// seconds. None unless both timestamps parse.
fn sun_times(sunrise: &str, sunset: &str, tz: &str) -> Option<(String, String, i64)> {
    use chrono::{DateTime, TimeZone};
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    let parse = |iso: &str| -> Option<(String, i64)> {
        let dt = DateTime::parse_from_rfc3339(iso).ok()?;
        let epoch = dt.timestamp();
        let local = zone.timestamp_opt(epoch, 0).single()?;
        Some((local.format("%H:%M").to_string(), epoch))
    };
    let (sr, sr_e) = parse(sunrise)?;
    let (ss, ss_e) = parse(sunset)?;
    Some((sr, ss, ss_e - sr_e))
}

fn print_help(cfg: &Config) {
    let current = cfg.get("weather_place").unwrap_or("unset");
    println!("Usage: imgw [-p|--place \"City,CC\"|\"lat,lon\"] [location]");
    println!("  Show the IMGW point forecast for a location, with active warnings.");
    println!("  With no argument, uses `weather_place` from the config file (currently: {current}).");
    println!("  Examples: imgw -p \"Warsaw,PL\"   |   imgw \"52.24,21.03\"");
    if let Some(p) = Config::path() {
        println!("  Config: {}", p.display());
    }
}

/// Parsed command line: `-p/--place VALUE`, `-h/--help`, and a bare positional.
struct Args {
    place: Option<String>,
    positional: Option<String>,
    help: bool,
}

impl Args {
    fn parse<I: Iterator<Item = String>>(mut it: I) -> Result<Args, String> {
        let mut place = None;
        let mut positional = None;
        let mut help = false;
        while let Some(a) = it.next() {
            match a.as_str() {
                "-h" | "--help" => help = true,
                "-p" | "--place" => {
                    place = Some(it.next().ok_or("Error: -p/--place needs a value")?);
                }
                s if s.starts_with("--place=") => place = Some(s["--place=".len()..].to_string()),
                s if s.starts_with("-p=") => place = Some(s["-p=".len()..].to_string()),
                s if s.starts_with('-') && s != "-" => {
                    return Err(format!("Error: unknown option `{s}`"));
                }
                s => {
                    if positional.is_none() {
                        positional = Some(s.to_string());
                    }
                }
            }
        }
        Ok(Args {
            place,
            positional,
            help,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn place_flag_forms() {
        assert_eq!(parse(&["-p", "Warsaw,PL"]).place.as_deref(), Some("Warsaw,PL"));
        assert_eq!(parse(&["--place", "Krakow"]).place.as_deref(), Some("Krakow"));
        assert_eq!(parse(&["--place=Gdansk"]).place.as_deref(), Some("Gdansk"));
    }

    #[test]
    fn positional_and_help() {
        assert_eq!(parse(&["52.24,21.03"]).positional.as_deref(), Some("52.24,21.03"));
        assert!(parse(&["-h"]).help);
        assert!(parse(&["--help"]).help);
    }

    #[test]
    fn missing_place_value_is_an_error() {
        assert!(Args::parse(["-p".to_string()].into_iter()).is_err());
    }

    #[test]
    fn unknown_option_is_an_error() {
        assert!(Args::parse(["--bogus".to_string()].into_iter()).is_err());
    }
}
