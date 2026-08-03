//! End-to-end integration tests: drive `run_place` with every external endpoint
//! pointed at a single mock server, and assert on the captured output. Covers the
//! happy path (forecast + both warning boxes), the no-warnings path, the fallback
//! token path, and a hard failure.

use imgw_rs::client::{Client, Endpoints};
use imgw_rs::http::Backoff;
use imgw_rs::ui::Colors;
use mockito::{Matcher, Server, ServerGuard};
use std::path::PathBuf;

/// A fresh, empty cache dir per test so token/basin caches don't leak between runs.
fn temp_cache(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("imgw-rs-it-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Endpoints for a mock server: each service gets a distinct path prefix so they don't
/// collide on the one host.
fn endpoints_for(server: &ServerGuard) -> Endpoints {
    let base = server.url();
    Endpoints {
        meteo: format!("{base}/meteo"),
        danepubliczne: format!("{base}/dane"),
        open_meteo: format!("{base}/om"),
        geocoding: format!("{base}/geo"),
        bigdatacloud: format!("{base}/bdc"),
    }
}

const FORECAST: &str = r#"{"data":{"Valid":true,
  "Sun":{"Sunrise":"2026-08-03T03:00:00Z","Sunset":"2026-08-03T18:00:00Z"},
  "Data":[{"Type":"Type_Ten_Minutes","Date":"2026-08-03T10:00:00Z",
    "Temperature":"300.05","Chill":"301.05","Wind_Speed":"3.0","Wind_Dir":"90",
    "Wind_Gust":"6.0","Humidity":"40","PressureMSL":"101500","Cloud":"10",
    "Icon10":"n0z00d","Rain10m":"0.0","Snow10m":"0.0","Precipitation10m":"0.0"}]}}"#;

const ZLEW: &str = r#"{"type":"FeatureCollection","features":[
  {"type":"Feature","properties":{"KOD":"K1","NAZWA":"TestBasin"},
   "geometry":{"type":"Polygon","coordinates":[[[19.0,49.0],[21.0,49.0],[21.0,51.0],[19.0,51.0],[19.0,49.0]]]}}]}"#;

/// Register the token-scrape + forecast mocks common to every happy-path test. Returns
/// the guards so the caller keeps them alive for the duration of the test.
fn mock_forecast(server: &mut Server) -> Vec<mockito::Mock> {
    vec![
        server
            .mock("GET", "/meteo/")
            .match_query(Matcher::Any)
            .with_body(r#"<script src="main.abc123.js"></script>"#)
            .create(),
        server
            .mock("GET", "/meteo/main.abc123.js")
            .match_query(Matcher::Any)
            .with_body(r#"window.x={apiToken:"TESTTOKEN"};"#)
            .create(),
        server
            .mock("GET", "/meteo/api/v1/forecast/fcapi")
            .match_query(Matcher::Any)
            .with_body(FORECAST)
            .create(),
    ]
}

#[test]
fn happy_path_renders_forecast_and_both_warning_boxes() {
    let mut server = Server::new();
    let cache = temp_cache("happy");

    let mut guards = mock_forecast(&mut server);
    guards.push(
        server
            .mock("GET", "/bdc")
            .match_query(Matcher::Any)
            .with_body(r#"{"city":"Krakow"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/om")
            .match_query(Matcher::Any)
            .with_body(r#"{"timezone":"Europe/Warsaw"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/api/v1/geo/search-reverse")
            .match_query(Matcher::Any)
            .with_body(r#"{"data":[{"teryt":"1261","dist":"0.5"}]}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/dane/warningsmeteo")
            .match_query(Matcher::Any)
            .with_body(
                r#"[{"nazwa_zdarzenia":"Upał","stopien":"3","obowiazuje_od":"2026-08-04 12:00:00","obowiazuje_do":"2026-08-06 20:00:00","teryt":["1261","1201"]}]"#,
            )
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/dyn/data/zlew.json")
            .match_query(Matcher::Any)
            .with_body(ZLEW)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/dane/warningshydro")
            .match_query(Matcher::Any)
            .with_body(
                r#"[{"stopień":"-1","zdarzenie":"Susza hydrologiczna","obszary":[{"kod_zlewni":["K1"]}]}]"#,
            )
            .create(),
    );

    let client = Client::with(endpoints_for(&server), Backoff::none());
    let mut out = Vec::new();
    let code = imgw_rs::run_place(
        &client,
        "50.06,19.94",
        Some(cache.as_path()),
        &Colors::plain(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();

    assert_eq!(code, 0, "output was:\n{text}");
    // meteo warning box
    assert!(text.contains("WARNING!"), "{text}");
    assert!(text.contains("Upał — level 3, from 2026-08-04 12:00:00 until 2026-08-06 20:00:00"), "{text}");
    // drought box, filtered to the point's basin
    assert!(text.contains("NOTICE"), "{text}");
    assert!(text.contains("Susza hydrologiczna (hydrological drought) — TestBasin basin"), "{text}");
    // forecast block
    assert!(text.contains("Weather in Krakow"), "{text}");
    assert!(text.contains("sunny"), "{text}");
    assert!(text.contains("Wind: 3.0 m/s E (90°), gust 6.0 m/s"), "{text}");
    assert!(text.contains("Sunrise: 05:00   Sunset: 20:00   (day 15h 00m)"), "{text}");
}

#[test]
fn splits_warnings_into_per_level_boxes_in_severity_order() {
    let mut server = Server::new();
    let cache = temp_cache("levels");

    let mut guards = mock_forecast(&mut server);
    guards.push(
        server
            .mock("GET", "/bdc")
            .match_query(Matcher::Any)
            .with_body(r#"{"city":"Krakow"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/om")
            .match_query(Matcher::Any)
            .with_body(r#"{"timezone":"Europe/Warsaw"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/api/v1/geo/search-reverse")
            .match_query(Matcher::Any)
            .with_body(r#"{"data":[{"teryt":"1261","dist":"0.5"}]}"#)
            .create(),
    );
    // Two meteo warnings, different levels -> two separate boxes.
    guards.push(
        server
            .mock("GET", "/dane/warningsmeteo")
            .match_query(Matcher::Any)
            .with_body(
                r#"[{"nazwa_zdarzenia":"Upał","stopien":"3","obowiazuje_od":"2026-08-04 12:00:00","obowiazuje_do":"2026-08-06 20:00:00","teryt":["1261"]},
                    {"nazwa_zdarzenia":"Upał","stopien":"2","obowiazuje_od":"2026-08-03 12:00:00","obowiazuje_do":"2026-08-03 20:00:00","teryt":["1261"]}]"#,
            )
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/dyn/data/zlew.json")
            .match_query(Matcher::Any)
            .with_body(ZLEW)
            .create(),
    );
    // A regular hydro warning (level 1, basin K1) plus a susza (level -1, basin K1).
    guards.push(
        server
            .mock("GET", "/dane/warningshydro")
            .match_query(Matcher::Any)
            .with_body(
                r#"[{"stopień":"1","zdarzenie":"Wezbranie","data_od":"2026-08-03 18:00:00","data_do":"2026-08-04 06:00:00","obszary":[{"kod_zlewni":["K1"]}]},
                    {"stopień":"-1","zdarzenie":"Susza hydrologiczna","obszary":[{"kod_zlewni":["K1"]}]}]"#,
            )
            .create(),
    );

    let client = Client::with(endpoints_for(&server), Backoff::none());
    let mut out = Vec::new();
    let code = imgw_rs::run_place(
        &client,
        "50.06,19.94",
        Some(cache.as_path()),
        &Colors::plain(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{text}");

    // Four boxes: meteo level 3, meteo level 2, hydro level 1, susza notice — in order.
    let i_l3 = text.find("Upał — level 3").expect("level 3 box");
    let i_l2 = text.find("Upał — level 2").expect("level 2 box");
    let i_hydro = text.find("Wezbranie — level 1").expect("hydro level 1 box");
    let i_susza = text.find("Susza hydrologiczna (hydrological drought)").expect("susza notice");
    assert!(i_l3 < i_l2, "level 3 must precede level 2\n{text}");
    assert!(i_l2 < i_hydro, "meteo must precede hydro\n{text}");
    assert!(i_hydro < i_susza, "hydro warnings must precede the drought notice\n{text}");
    assert_eq!(text.matches("WARNING!").count(), 3, "3 warning boxes\n{text}");
    assert_eq!(text.matches("NOTICE").count(), 1, "1 notice box\n{text}");
}

#[test]
fn no_warnings_renders_only_the_forecast() {
    let mut server = Server::new();
    let cache = temp_cache("nowarn");

    let mut guards = mock_forecast(&mut server);
    guards.push(
        server
            .mock("GET", "/bdc")
            .match_query(Matcher::Any)
            .with_body(r#"{"city":"Krakow"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/om")
            .match_query(Matcher::Any)
            .with_body(r#"{"timezone":"Europe/Warsaw"}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/api/v1/geo/search-reverse")
            .match_query(Matcher::Any)
            .with_body(r#"{"data":[{"teryt":"1261","dist":"0.5"}]}"#)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/dane/warningsmeteo")
            .match_query(Matcher::Any)
            .with_body("[]")
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/meteo/dyn/data/zlew.json")
            .match_query(Matcher::Any)
            .with_body(ZLEW)
            .create(),
    );
    guards.push(
        server
            .mock("GET", "/dane/warningshydro")
            .match_query(Matcher::Any)
            .with_body("[]")
            .create(),
    );

    let client = Client::with(endpoints_for(&server), Backoff::none());
    let mut out = Vec::new();
    let code = imgw_rs::run_place(
        &client,
        "50.06,19.94",
        Some(cache.as_path()),
        &Colors::plain(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();

    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("WARNING!"), "{text}");
    assert!(!text.contains("NOTICE"), "{text}");
    assert!(text.contains("Weather in Krakow"), "{text}");
}

#[test]
fn falls_back_to_the_builtin_token_when_scraping_fails() {
    let mut server = Server::new();
    let cache = temp_cache("fallback");

    // Token scrape fails (root 500), so the built-in fallback token is used and the
    // forecast still succeeds. No warnings, to keep the mock set small.
    let _root = server
        .mock("GET", "/meteo/")
        .match_query(Matcher::Any)
        .with_status(500)
        .create();
    let _fc = server
        .mock("GET", "/meteo/api/v1/forecast/fcapi")
        .match_query(Matcher::Any)
        .with_body(FORECAST)
        .create();
    let _bdc = server
        .mock("GET", "/bdc")
        .match_query(Matcher::Any)
        .with_body(r#"{"city":"Krakow"}"#)
        .create();
    let _om = server
        .mock("GET", "/om")
        .match_query(Matcher::Any)
        .with_body(r#"{"timezone":"Europe/Warsaw"}"#)
        .create();
    // reverse + warnings feeds empty / absent -> no boxes.
    let _rev = server
        .mock("GET", "/meteo/api/v1/geo/search-reverse")
        .match_query(Matcher::Any)
        .with_body(r#"{"data":[]}"#)
        .create();
    let _zlew = server
        .mock("GET", "/meteo/dyn/data/zlew.json")
        .match_query(Matcher::Any)
        .with_body(r#"{"type":"FeatureCollection","features":[]}"#)
        .create();

    let client = Client::with(endpoints_for(&server), Backoff::none());
    let mut out = Vec::new();
    let code = imgw_rs::run_place(
        &client,
        "50.06,19.94",
        Some(cache.as_path()),
        &Colors::plain(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();

    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Weather in Krakow"), "{text}");
}

#[test]
fn reports_failure_when_the_forecast_is_unavailable() {
    let mut server = Server::new();
    let cache = temp_cache("fail");

    // Geocoding works, but every forecast attempt (and token scrape) 500s.
    let _bdc = server
        .mock("GET", "/bdc")
        .match_query(Matcher::Any)
        .with_body(r#"{"city":"Krakow"}"#)
        .create();
    let _om = server
        .mock("GET", "/om")
        .match_query(Matcher::Any)
        .with_body(r#"{"timezone":"Europe/Warsaw"}"#)
        .create();
    let _root = server
        .mock("GET", "/meteo/")
        .match_query(Matcher::Any)
        .with_status(500)
        .create();
    let _fc = server
        .mock("GET", "/meteo/api/v1/forecast/fcapi")
        .match_query(Matcher::Any)
        .with_status(500)
        .create();

    let client = Client::with(endpoints_for(&server), Backoff::none());
    let mut out = Vec::new();
    let code = imgw_rs::run_place(
        &client,
        "50.06,19.94",
        Some(cache.as_path()),
        &Colors::plain(),
        &mut out,
    );

    assert_eq!(code, 1);
    assert!(String::from_utf8(out).unwrap().is_empty());
}
