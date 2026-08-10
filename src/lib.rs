//! `pogoda` — a terminal point-weather nowcast from IMGW-PIB, the Polish
//! meteorological service. Resolves a location to coordinates, fetches the HYBRID
//! point forecast, and prints it (in Polish) with any active meteorological and
//! hydrological (drought) warnings for that exact point.

pub mod client;
pub mod config;
pub mod forecast;
pub mod geo;
pub mod http;
pub mod imgw;
pub mod model;
pub mod text;
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

    // The positional location, else the configured default.
    let place = args
        .positional
        .or_else(|| cfg.get("weather_place").map(String::from));
    let place = match place {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            eprintln!("{}", text::ERR_NO_LOCATION);
            eprintln!("{}", text::USAGE);
            eprintln!("{}", text::EXAMPLE);
            return 1;
        }
    };

    // Forecast horizon: cap at end of tomorrow (default), unless `forecast_horizon = full`.
    let horizon = match cfg.get("forecast_horizon") {
        Some(v) if v.trim().eq_ignore_ascii_case("full") => full_horizon(),
        _ => end_of_next_day_utc(),
    };

    let client = Client::new();
    let cache = config::cache_dir();
    let colors = Colors::detect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    run_place(
        &client,
        &place,
        cache.as_deref(),
        &colors,
        horizon,
        &mut out,
    )
}

/// A horizon far enough ahead that no forecast step is trimmed (`forecast_horizon = full`).
fn full_horizon() -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(9999, 1, 1, 0, 0, 0).unwrap()
}

/// Fetch and render the forecast (with warnings) for `place`, writing to `out`. Returns
/// a process exit code; user-facing errors go to stderr. `horizon_end` caps how far the
/// forecast reaches (steps beyond it are dropped). This is the testable core: point
/// `client`'s endpoints at a mock server and capture `out`.
#[allow(clippy::too_many_arguments)]
pub fn run_place(
    client: &Client,
    place: &str,
    cache: Option<&Path>,
    colors: &Colors,
    horizon_end: chrono::DateTime<chrono::Utc>,
    out: &mut impl Write,
) -> i32 {
    // Wave 1 — everything that needs neither the token nor the coordinates runs
    // concurrently: location resolution, the app token (scrape, else fall back), both
    // (nationwide) warning feeds, and the river-basin polygons. `thread::scope` lets the
    // threads borrow `client` etc. and guarantees they're all joined before it returns,
    // so no `'static`/`Arc` is needed and the compiler proves there are no data races.
    let (loc_res, token, meteo_raw, hydro_raw, basins) = std::thread::scope(|s| {
        let loc = s.spawn(|| geo::resolve(client, place));
        let token = s.spawn(|| {
            cache
                .and_then(|d| imgw::token(client, d, false).ok())
                .unwrap_or_else(|| imgw::FALLBACK_TOKEN.to_string())
        });
        let meteo = s.spawn(|| imgw::warning_feed(client, "warningsmeteo"));
        let hydro = s.spawn(|| imgw::warning_feed(client, "warningshydro"));
        let basins = s.spawn(|| cache.and_then(|d| imgw::ensure_basins(client, d)));
        (
            loc.join().unwrap(),
            token.join().unwrap(),
            meteo.join().unwrap(),
            hydro.join().unwrap(),
            basins.join().unwrap(),
        )
    });

    let loc = match loc_res {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}: {e}", text::PROGRAM);
            return 1;
        }
    };

    // Wave 2 — the two calls that need the token (and coordinates) run concurrently:
    // the point forecast and the area-code lookup that filters the meteo feed.
    let mut token = token;
    let (mut body, mut area) = fetch_forecast_and_area(client, &token, &loc);

    // On an empty forecast the cached token may be stale: refresh once and re-run
    // wave 2 with the fresh token (rare path).
    if body.is_none() {
        if let Some(d) = cache {
            if let Ok(fresh) = imgw::token(client, d, true) {
                token = fresh;
                let (body2, area2) = fetch_forecast_and_area(client, &token, &loc);
                body = body2;
                if area.is_none() {
                    area = area2;
                }
            }
        }
    }

    let body = match body {
        Some(b) => b,
        None => {
            eprintln!("{}: {}", text::PROGRAM, text::FORECAST_FAILED);
            return 1;
        }
    };

    let fc = match forecast::parse(&body, horizon_end) {
        Ok(f) => f,
        Err(_) => {
            eprintln!(
                "{}: {}",
                text::PROGRAM,
                text::no_forecast_data(&loc.label, loc.lat, loc.lon)
            );
            return 1;
        }
    };

    // Render (sequential, ordered) from the already-fetched data: warning boxes above
    // the forecast, then the forecast itself. The banner line sets the shared box width
    // so every warning box aligns to it.
    let today = chrono::Utc::now()
        .with_timezone(&chrono_tz::Europe::Warsaw)
        .date_naive();
    let banner = forecast_banner(&fc, &loc);
    let box_width = banner.chars().count();
    render_warnings(
        colors,
        today,
        &loc,
        area.as_deref(),
        meteo_raw.as_deref(),
        hydro_raw.as_deref(),
        basins.as_deref(),
        box_width,
        out,
    );
    print_forecast(&fc, &loc, &banner, out);

    // Source attribution, under every forecast.
    let _ = writeln!(out, "\n{}", text::SOURCE);

    0
}

/// Forecast horizon: the end of *tomorrow* in Poland, as a UTC instant. IMGW's step
/// timestamps are UTC (the trailing `Z`), but "end of tomorrow" is a Warsaw wall-clock
/// boundary — so we express that boundary in UTC to compare against the step times. The
/// single conversion is unavoidable; it's not decoration. (Poland's DST switch is at
/// 02:00/03:00, never midnight, so the local→UTC mapping is always unambiguous.)
fn end_of_next_day_utc() -> chrono::DateTime<chrono::Utc> {
    use chrono::{Duration, TimeZone, Utc};
    let tz = chrono_tz::Europe::Warsaw;
    let midnight_after_tomorrow = (Utc::now().with_timezone(&tz).date_naive() + Duration::days(2))
        .and_hms_opt(0, 0, 0)
        .unwrap();
    tz.from_local_datetime(&midnight_after_tomorrow)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

/// Wave 2: fetch the point forecast (non-empty body) and the area code concurrently,
/// both using `token`. Returns `(forecast_body, area_code)`.
fn fetch_forecast_and_area(
    client: &Client,
    token: &str,
    loc: &geo::Location,
) -> (Option<String>, Option<String>) {
    std::thread::scope(|s| {
        let body = s.spawn(|| {
            imgw::forecast(client, token, loc.lat, loc.lon)
                .ok()
                .filter(|b| !b.trim().is_empty())
        });
        let area = s.spawn(|| imgw::area_code(client, token, loc.lat, loc.lon));
        (body.join().unwrap(), area.join().unwrap())
    })
}

/// The forecast banner line (" Pogoda: … °C …"). Its width is reused as the shared
/// width for the warning boxes so everything lines up.
fn forecast_banner(fc: &forecast::Forecast, loc: &geo::Location) -> String {
    let cond = weather::condition(&fc.icon, fc.rain, fc.snow, fc.prec);
    let cond_suffix = if cond.is_empty() {
        String::new()
    } else {
        format!(", {cond}")
    };
    text::banner(&loc.label, fc.temp, &cond_suffix, fc.feels)
}

/// Render one warning as a coloured box: level + probability in the caption, the
/// event/window headline first, then the wrapped description (if any). `inner_width` is
/// the shared box interior width; the description wraps to fit inside it.
fn emit_warning_box(
    w: &warnings::Warning,
    colors: &Colors,
    inner_width: usize,
    out: &mut impl Write,
) {
    let caption = text::warning_caption(w.level, &w.prob);
    let mut body = vec![w.headline.clone()];
    if !w.desc.is_empty() {
        body.extend(ui::wrap(&w.desc, inner_width.saturating_sub(2)));
    }
    for l in ui::warn_box(
        colors.level(w.level),
        &colors.reset,
        &caption,
        &body,
        inner_width,
    ) {
        let _ = writeln!(out, "{l}");
    }
}

/// Render the warning boxes from already-fetched data (pure: no network). `area` is the
/// point's administrative-area code, `meteo_raw`/`hydro_raw` the raw feed bodies, and
/// `basins` the path to the basin polygons. Meteo boxes first, then hydro boxes, then
/// the drought notice.
#[allow(clippy::too_many_arguments)]
fn render_warnings(
    colors: &Colors,
    today: chrono::NaiveDate,
    loc: &geo::Location,
    area: Option<&str>,
    meteo_raw: Option<&str>,
    hydro_raw: Option<&str>,
    basins: Option<&Path>,
    box_width: usize,
    out: &mut impl Write,
) {
    // Shared box interior width: box total = inner + 2 borders, so inner = box_width - 2
    // makes every box exactly as wide as the forecast banner.
    let inner = box_width.saturating_sub(2);

    // Meteo warnings, filtered to this point's administrative area: one box per
    // warning, coloured by level (3=red, 2=orange, 1=yellow), highest severity first.
    if let (Some(area), Some(raw)) = (area, meteo_raw) {
        if let Ok(items) = serde_json::from_str::<Vec<model::MeteoWarning>>(raw) {
            let ws = warnings::ordered_for_display(warnings::meteo_warnings(&items, area, today));
            for w in &ws {
                emit_warning_box(w, colors, inner, out);
            }
        }
    }

    // Hydrological warnings, filtered to this point's river basin: regular warnings
    // (levels 1/2/3) as coloured boxes, then the drought (susza, level -1) as a grey
    // notice.
    if let (Some(basins), Some(raw)) = (basins, hydro_raw) {
        if let Some(basin) = warnings::find_basin(basins, loc.lat, loc.lon) {
            if let Ok(items) = serde_json::from_str::<Vec<model::HydroWarning>>(raw) {
                let ws = warnings::ordered_for_display(warnings::hydro_warnings(
                    &items,
                    &basin.code,
                    today,
                ));
                for w in &ws {
                    emit_warning_box(w, colors, inner, out);
                }
                if warnings::drought_hits_basin(&items, &basin.code) {
                    let line = text::drought_notice(&basin.name);
                    for l in ui::warn_box(
                        &colors.grey,
                        &colors.reset,
                        text::NOTICE_CAPTION,
                        &[line],
                        inner,
                    ) {
                        let _ = writeln!(out, "{l}");
                    }
                }
            }
        }
    }
}

/// Write the forecast block: `banner` (prebuilt), then precip, wind, pressure,
/// humidity, cloud, sun.
fn print_forecast(
    fc: &forecast::Forecast,
    loc: &geo::Location,
    banner: &str,
    out: &mut impl Write,
) {
    let _ = writeln!(out, "{banner}");

    let h = fc.hrs.round() as i64;
    if fc.prec_sum > 0.0 {
        let kind = if fc.rain_sum > 0.0 && fc.snow_sum > 0.0 {
            text::PRECIP_RAIN_AND_SNOW
        } else if fc.snow_sum > 0.0 {
            text::PRECIP_SNOW
        } else if fc.rain_sum > 0.0 {
            text::PRECIP_RAIN
        } else {
            text::PRECIP_MIXED
        };
        // At most two clauses, split on whether it's raining now (onset ≈ 0):
        //   raining now → rain until it stops, then the dry tail (if any)
        //   dry now     → dry until it starts, then rain over the rest of the window
        let onset = fc.onset_hours.unwrap_or(0.0).round() as i64;
        let line = if onset == 0 {
            // Raining now: classify the current rate (rain only; snow/mixed keep their
            // label), then the amount until it stops, then the dry tail.
            let prefix = if kind == text::PRECIP_RAIN {
                text::rain_intensity(fc.prec_rate_mmh).to_string()
            } else {
                format!("{kind}:")
            };
            let end = fc.precip_end_hours.unwrap_or(fc.hrs).round() as i64;
            if h - end >= 1 {
                let rain_h = end.max(1);
                text::rain_then_dry(&prefix, fc.prec_sum, rain_h, Some(h - rain_h))
            } else {
                text::rain_then_dry(&prefix, fc.prec_sum, h, None)
            }
        } else {
            text::dry_then_rain(kind, onset, fc.prec_sum, (h - onset).max(1))
        };
        let _ = writeln!(out, "{line}");
    } else {
        let _ = writeln!(out, "{}", text::precip_none(h));
    }

    if !fc.wspeed.is_empty() {
        let card = weather::cardinal(&fc.wdir);
        let gust = (!fc.gust.is_empty()).then_some(fc.gust.as_str());
        let _ = writeln!(
            out,
            "{}",
            text::wind_line(&fc.wspeed, &card, &fc.wdir, gust)
        );
    }
    if fc.pres > 0.0 {
        let _ = writeln!(out, "{}", text::pressure_line(fc.pres));
    }
    if !fc.hum.is_empty() {
        let _ = writeln!(out, "{}", text::humidity_line(&fc.hum));
    }
    if !fc.cloud.is_empty() {
        let _ = writeln!(out, "{}", text::cloud_line(&fc.cloud));
    }
    if let Some((sr, ss, daysec)) = sun_times(&fc.sunrise, &fc.sunset, &loc.tz) {
        let _ = writeln!(
            out,
            "{}",
            text::sun_line(&sr, &ss, daysec / 3600, (daysec % 3600) / 60)
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
    let current = cfg.get("weather_place").unwrap_or(text::UNSET);
    println!("{}", text::USAGE);
    println!("{}", text::HELP_DESC);
    println!("{}", text::help_default(current));
    println!("{}", text::HELP_EXAMPLES);
    if let Some(p) = Config::path() {
        println!("{}", text::help_config(&p.display().to_string()));
    }
}

/// Parsed command line: a bare positional location, and `-h/--help`.
struct Args {
    positional: Option<String>,
    help: bool,
}

impl Args {
    fn parse<I: Iterator<Item = String>>(it: I) -> Result<Args, String> {
        let mut positional = None;
        let mut help = false;
        for a in it {
            match a.as_str() {
                "-h" | "--help" => help = true,
                s if s.starts_with('-') && s != "-" => {
                    return Err(text::err_unknown_option(s));
                }
                s => {
                    if positional.is_none() {
                        positional = Some(s.to_string());
                    }
                }
            }
        }
        Ok(Args { positional, help })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn positional_and_help() {
        assert_eq!(
            parse(&["52.24,21.03"]).positional.as_deref(),
            Some("52.24,21.03")
        );
        assert_eq!(
            parse(&["Warszawa,PL"]).positional.as_deref(),
            Some("Warszawa,PL")
        );
        assert!(parse(&["-h"]).help);
        assert!(parse(&["--help"]).help);
    }

    #[test]
    fn first_positional_wins() {
        assert_eq!(
            parse(&["Kraków", "Gdańsk"]).positional.as_deref(),
            Some("Kraków")
        );
    }

    #[test]
    fn unknown_option_is_an_error() {
        assert!(Args::parse(["--bogus".to_string()].into_iter()).is_err());
    }
}
