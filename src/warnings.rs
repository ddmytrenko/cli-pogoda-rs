//! Warning logic. Two independent box kinds:
//!
//!  * meteo — the nationwide meteo feed filtered to this point's administrative area
//!    (each warning is tagged with the area codes it covers).
//!  * hydrological — locate this point's river basin by point-in-polygon over the
//!    basin polygons, then match its code against the active hydro warnings, so a
//!    warning elsewhere in the country does not raise a box here.
//!
//! The point-in-polygon test and both filters are pure and unit-tested; the network
//! fetches live in imgw.rs.

use crate::model::{BasinCollection, HydroWarning, MeteoWarning};
use crate::text;
use chrono::NaiveDate;
use serde_json::Value;
use std::path::Path;

/// Wire values for the hydrological-drought special case: IMGW encodes it as a
/// level `-1` warning whose event name contains "susza" (Polish for "drought").
const DROUGHT_LEVEL: &str = "-1";
const DROUGHT_EVENT_MARKER: &str = "susza";

/// Ray-casting point-in-polygon on a ring of `[lon, lat]` pairs.
pub fn point_in_ring(ring: &[Vec<f64>], lon: f64, lat: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let pi = &ring[i];
        let pj = &ring[(i + n - 1) % n];
        if pi.len() < 2 || pj.len() < 2 {
            continue;
        }
        let (xi, yi) = (pi[0], pi[1]);
        let (xj, yj) = (pj[0], pj[1]);
        if (yi > lat) != (yj > lat) {
            let denom = if (yj - yi) == 0.0 { 1e-15 } else { yj - yi };
            if lon < (xj - xi) * (lat - yi) / denom + xi {
                inside = !inside;
            }
        }
    }
    inside
}

/// A single warning, rendered as its own box: `level` + `prob` go in the caption,
/// `headline` is the first body line, `desc` (if any) wraps below it. `from`/`until`
/// are the trimmed raw timestamps kept as the display sort key.
#[derive(Debug, PartialEq, Eq)]
pub struct Warning {
    pub level: i64,
    pub prob: String, // probability without "%", "" if none
    pub from: String,
    pub until: String,
    pub headline: String, // "<event> — from <start> until <end>"
    pub desc: String,     // free-text description, "" if none
}

fn parse_level(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// Merge a warning's main description with its remarks, dropping remarks that are
/// absent or "Brak" (the feed's value for "none"). When both are present the remarks go
/// in their own paragraph (blank line between). Whitespace-trimmed.
fn merge_desc(main: &str, comment: &str) -> String {
    let main = main.trim();
    let comment = comment.trim();
    let comment_is_none =
        comment.is_empty() || comment.trim_end_matches('.').eq_ignore_ascii_case("brak");
    match (main.is_empty(), comment_is_none) {
        (_, true) => main.to_string(),
        (true, false) => comment.to_string(),
        (false, false) => format!("{main}\n\n{comment}"),
    }
}

/// Trim seconds off an IMGW timestamp: "YYYY-MM-DD HH:MM:SS" -> "YYYY-MM-DD HH:MM".
/// Anything not in that exact shape is returned unchanged.
fn drop_seconds(ts: &str) -> &str {
    if ts.len() == 19 && ts.as_bytes()[16] == b':' {
        &ts[..16]
    } else {
        ts
    }
}

/// Render a trimmed "YYYY-MM-DD HH:MM" timestamp relative to `today`: just "HH:MM" if
/// it's today, "yesterday HH:MM" / "tomorrow HH:MM" for the adjacent days, otherwise
/// the full "YYYY-MM-DD HH:MM". Non-conforming input is returned unchanged.
fn humanize_ts(ts: &str, today: NaiveDate) -> String {
    if ts.len() < 16 {
        return ts.to_string();
    }
    let date_part = &ts[..10];
    let time = &ts[11..16];
    match NaiveDate::parse_from_str(date_part, "%Y-%m-%d") {
        Ok(d) => match (d - today).num_days() {
            0 => time.to_string(),
            -1 => format!("{} {time}", text::YESTERDAY),
            1 => format!("{} {time}", text::TOMORROW),
            _ => ts.to_string(),
        },
        Err(_) => ts.to_string(),
    }
}

/// Meteo warnings for this point's administrative `area`, one per warning that covers
/// it, each tagged with its severity level (1/2/3) for colour grouping. `today` (in the
/// timestamps' timezone) drives the relative "today/yesterday/tomorrow" formatting.
pub fn meteo_warnings(warnings: &[MeteoWarning], area: &str, today: NaiveDate) -> Vec<Warning> {
    warnings
        .iter()
        .filter(|w| w.areas.iter().any(|c| c == area))
        .filter_map(|w| {
            let level = parse_level(&w.level)?;
            let from = drop_seconds(&w.valid_from);
            let until = drop_seconds(&w.valid_until);
            let desc = merge_desc(&w.description, &w.remarks);
            let (from_disp, until_disp) = (humanize_ts(from, today), humanize_ts(until, today));
            Some(Warning {
                level,
                prob: w.probability.clone(),
                from: from.to_string(),
                until: until.to_string(),
                headline: text::warning_headline(&w.event, &from_disp, &until_disp),
                desc,
            })
        })
        .collect()
}

/// Regular hydrological warnings (levels 1/2/3) for this point's river basin.
/// The hardcoded drought level (-1) is excluded — see [`drought_hits_basin`] for
/// that separate notice.
pub fn hydro_warnings(
    warnings: &[HydroWarning],
    basin_code: &str,
    today: NaiveDate,
) -> Vec<Warning> {
    warnings
        .iter()
        .filter_map(|w| {
            let level = parse_level(&w.level)?;
            if level < 1 {
                return None; // drought (-1) is handled as a separate notice
            }
            if !hydro_covers_basin(w, basin_code) {
                return None;
            }
            let from = drop_seconds(&w.valid_from);
            let until = drop_seconds(&w.valid_until);
            let desc = merge_desc(&w.description, &w.remarks);
            let (from_disp, until_disp) = (humanize_ts(from, today), humanize_ts(until, today));
            Some(Warning {
                level,
                prob: w.probability.clone(),
                from: from.to_string(),
                until: until.to_string(),
                headline: text::warning_headline(&w.event, &from_disp, &until_disp),
                desc,
            })
        })
        .collect()
}

/// Whether a hydro warning's affected areas include the basin `basin_code`.
fn hydro_covers_basin(w: &HydroWarning, basin_code: &str) -> bool {
    w.areas
        .iter()
        .any(|a| a.basin_codes.iter().any(|c| c == basin_code))
}

/// Order warnings for display: highest severity first, and within a severity
/// closest-first (earliest start, then earliest end — the ISO-like timestamps sort
/// chronologically as plain strings). Each warning is rendered as its own box.
pub fn ordered_for_display(mut warnings: Vec<Warning>) -> Vec<Warning> {
    warnings.sort_by(|a, b| {
        b.level
            .cmp(&a.level)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.until.cmp(&b.until))
    });
    warnings
}

/// A river basin, identified by its code and human name.
pub struct Basin {
    pub code: String,
    pub name: String,
}

/// Locate the river basin containing (lat, lon) by point-in-polygon over the basin
/// polygons. Returns the first matching feature's (code, name).
pub fn find_basin(basins_path: &Path, lat: f64, lon: f64) -> Option<Basin> {
    let text = std::fs::read_to_string(basins_path).ok()?;
    let collection: BasinCollection = serde_json::from_str(&text).ok()?;
    for f in &collection.features {
        // Normalise to a list of polygons, each a list of rings (exterior first). The
        // coordinate arrays stay untyped: GeoJSON nests them by geometry type.
        let polys: Vec<&Value> = match f.geometry.kind.as_str() {
            "Polygon" => vec![&f.geometry.coordinates],
            "MultiPolygon" => f
                .geometry
                .coordinates
                .as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default(),
            _ => continue,
        };
        let hit = polys.iter().any(|poly| {
            poly.get(0)
                .and_then(ring_to_vec)
                .map(|ring| point_in_ring(&ring, lon, lat))
                .unwrap_or(false)
        });
        if hit {
            return Some(Basin {
                code: f.properties.code.clone(),
                name: f.properties.name.clone(),
            });
        }
    }
    None
}

fn ring_to_vec(ring: &Value) -> Option<Vec<Vec<f64>>> {
    let pts = ring.as_array()?;
    Some(
        pts.iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some(vec![a.first()?.as_f64()?, a.get(1)?.as_f64()?])
            })
            .collect(),
    )
}

/// True if a hydrological drought (level `-1`, or a drought-marker event) is active for
/// the basin `basin_code`, from the hydro-warning items.
pub fn drought_hits_basin(warnings: &[HydroWarning], basin_code: &str) -> bool {
    warnings.iter().any(|w| {
        let is_drought =
            w.level == DROUGHT_LEVEL || w.event.to_lowercase().contains(DROUGHT_EVENT_MARKER);
        is_drought && hydro_covers_basin(w, basin_code)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A unit square with corners (0,0)-(2,2); rings are [lon,lat] pairs, closed.
    fn square() -> Vec<Vec<f64>> {
        vec![
            vec![0.0, 0.0],
            vec![2.0, 0.0],
            vec![2.0, 2.0],
            vec![0.0, 2.0],
            vec![0.0, 0.0],
        ]
    }

    #[test]
    fn point_inside_and_outside_square() {
        let sq = square();
        assert!(point_in_ring(&sq, 1.0, 1.0)); // centre
        assert!(!point_in_ring(&sq, 3.0, 1.0)); // east, outside
        assert!(!point_in_ring(&sq, -1.0, 1.0)); // west, outside
        assert!(!point_in_ring(&sq, 1.0, 5.0)); // north, outside
    }

    #[test]
    fn degenerate_ring_is_never_inside() {
        assert!(!point_in_ring(&[vec![0.0, 0.0], vec![1.0, 1.0]], 0.5, 0.5));
    }

    #[test]
    fn meteo_warnings_filter_by_area_with_probability_and_description() {
        let warn: Vec<MeteoWarning> = serde_json::from_str(
            r#"[
              {"nazwa_zdarzenia":"Upał","stopien":"3","prawdopodobienstwo":"85","obowiazuje_od":"2026-08-04 12:00:00","obowiazuje_do":"2026-08-01 20:00:00","tresc":"Prognozuje się upały.","komentarz":"Brak.","teryt":["1206","1201"]},
              {"nazwa_zdarzenia":"Burze","stopien":"1","prawdopodobienstwo":"70","obowiazuje_od":"2026-07-31 15:00:00","obowiazuje_do":"2026-07-31 21:00:00","tresc":"Burze z gradem.","teryt":["1465"]}
            ]"#,
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let ws = meteo_warnings(&warn, "1206", today);
        assert_eq!(
            ws,
            vec![Warning {
                level: 3,
                prob: "85".into(),
                from: "2026-08-04 12:00".into(),  // tomorrow
                until: "2026-08-01 20:00".into(), // two days back -> absolute
                headline: "Upał — od jutra 12:00 do 2026-08-01 20:00".into(),
                desc: "Prognozuje się upały.".into(), // remarks "Brak." dropped
            }]
        );
        assert!(meteo_warnings(&warn, "9999", today).is_empty());
    }

    #[test]
    fn merge_desc_appends_meaningful_remarks_and_drops_brak() {
        assert_eq!(merge_desc("Upały.", "Brak."), "Upały.");
        assert_eq!(merge_desc("Upały.", "brak"), "Upały.");
        assert_eq!(merge_desc("Upały.", "  "), "Upały.");
        // both present -> remarks on their own paragraph (blank line between)
        assert_eq!(
            merge_desc("Upały.", "Możliwe podtopienia."),
            "Upały.\n\nMożliwe podtopienia."
        );
        assert_eq!(
            merge_desc("", "Możliwe podtopienia."),
            "Możliwe podtopienia."
        );
        assert_eq!(merge_desc("  Upały.  ", ""), "Upały.");
        // "brak" as a substring of a real remark is kept
        assert_eq!(merge_desc("X.", "Brak opadów."), "X.\n\nBrak opadów.");
    }

    fn w(level: i64, from: &str, until: &str, headline: &str) -> Warning {
        Warning {
            level,
            prob: String::new(),
            from: from.into(),
            until: until.into(),
            headline: headline.into(),
            desc: String::new(),
        }
    }

    #[test]
    fn drop_seconds_trims_only_the_full_timestamp_shape() {
        assert_eq!(drop_seconds("2026-08-06 20:00:00"), "2026-08-06 20:00");
        assert_eq!(drop_seconds("2026-08-06 20:00"), "2026-08-06 20:00"); // already trimmed
        assert_eq!(drop_seconds(""), "");
    }

    #[test]
    fn humanize_ts_uses_relative_days_around_today() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        assert_eq!(humanize_ts("2026-08-03 09:05", today), "09:05"); // today -> time only
        assert_eq!(humanize_ts("2026-08-02 23:59", today), "wczoraj 23:59");
        assert_eq!(humanize_ts("2026-08-04 06:00", today), "jutra 06:00");
        assert_eq!(humanize_ts("2026-08-06 20:00", today), "2026-08-06 20:00"); // further out
        assert_eq!(humanize_ts("2026-07-31 12:00", today), "2026-07-31 12:00"); // 3 days back
        assert_eq!(humanize_ts("", today), ""); // non-conforming
    }

    #[test]
    fn ordered_for_display_is_severity_desc_then_closest_first() {
        // Highest level first; within a level earliest start, then earliest end.
        let ws = vec![
            w(1, "2026-08-01 00:00", "2026-08-02 00:00", "l1"),
            w(2, "2026-08-05 00:00", "2026-08-06 00:00", "l2-late"),
            w(3, "2026-08-01 00:00", "2026-08-02 00:00", "l3"),
            w(
                2,
                "2026-08-03 00:00",
                "2026-08-09 00:00",
                "l2-early-lateend",
            ),
            w(
                2,
                "2026-08-03 00:00",
                "2026-08-04 00:00",
                "l2-early-earlyend",
            ),
        ];
        let ordered: Vec<String> = ordered_for_display(ws)
            .into_iter()
            .map(|w| w.headline)
            .collect();
        assert_eq!(
            ordered,
            vec![
                "l3",
                "l2-early-earlyend",
                "l2-early-lateend",
                "l2-late",
                "l1"
            ]
        );
    }

    #[test]
    fn hydro_warnings_filter_by_basin_and_exclude_drought() {
        let hydro: Vec<HydroWarning> = serde_json::from_str(
            r#"[
              {"stopień":"-1","zdarzenie":"Susza hydrologiczna","data_do":"2026-09-01 00:00:00",
               "obszary":[{"kod_zlewni":["R_K_MP_1"]}]},
              {"stopień":"2","zdarzenie":"Gwałtowne wzrosty stanów wody","prawdopodobienstwo":"80","data_od":"2026-08-03 14:10:00","data_do":"2026-08-03 22:00:00","przebieg":"Wzrosty stanów wody.","komentarz":"Możliwe podtopienia.",
               "obszary":[{"kod_zlewni":["R_K_MP_1","R_K_MP_9"]}]},
              {"stopień":"1","zdarzenie":"Wezbranie","data_od":"2026-08-03 18:00:00","data_do":"2026-08-04 06:00:00",
               "obszary":[{"kod_zlewni":["R_K_MP_2"]}]}
            ]"#,
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let ws = hydro_warnings(&hydro, "R_K_MP_1", today);
        // only the level-2 warning covers R_K_MP_1; drought (-1) is excluded here.
        // both timestamps are today -> shown as time only; description + remarks merged
        assert_eq!(
            ws,
            vec![Warning {
                level: 2,
                prob: "80".into(),
                from: "2026-08-03 14:10".into(),
                until: "2026-08-03 22:00".into(),
                headline: "Gwałtowne wzrosty stanów wody — od 14:10 do 22:00".into(),
                desc: "Wzrosty stanów wody.\n\nMożliwe podtopienia.".into(),
            }]
        );
    }

    #[test]
    fn drought_matches_basin_by_level_or_event() {
        let hydro: Vec<HydroWarning> = serde_json::from_str(
            r#"[
              {"stopień":"-1","zdarzenie":"Susza hydrologiczna",
               "obszary":[{"kod_zlewni":["O_Z_K_52","X_1"]}]},
              {"stopień":"2","zdarzenie":"Wezbranie",
               "obszary":[{"kod_zlewni":["Y_2"]}]}
            ]"#,
        )
        .unwrap();
        assert!(drought_hits_basin(&hydro, "O_Z_K_52"));
        assert!(!drought_hits_basin(&hydro, "Y_2")); // active area, but not a drought
        assert!(!drought_hits_basin(&hydro, "Z_9")); // not present at all
    }
}
