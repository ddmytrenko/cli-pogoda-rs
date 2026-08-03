//! Warning logic. Two independent boxes:
//!
//!  * meteo — the nationwide `warningsmeteo` feed filtered to this point's powiat
//!    TERYT (each warning is tagged with the powiat codes it covers).
//!  * hydrological drought — locate this point's river basin by point-in-polygon over
//!    zlew.json, then match its KOD against the active `warningshydro` drought areas,
//!    so a drought elsewhere in the country does not raise a box here.
//!
//! The point-in-polygon test and both filters are pure and unit-tested; the network
//! fetches live in imgw.rs.

use chrono::NaiveDate;
use serde_json::Value;
use std::path::Path;

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

/// A single warning: its severity `level`, validity window (`from`/`until`, kept for
/// sorting), and its rendered display `line`.
#[derive(Debug, PartialEq, Eq)]
pub struct Warning {
    pub level: i64,
    pub from: String,
    pub until: String,
    pub line: String,
}

fn parse_level(s: &str) -> Option<i64> {
    s.trim().parse().ok()
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
            -1 => format!("yesterday {time}"),
            1 => format!("tomorrow {time}"),
            _ => ts.to_string(),
        },
        Err(_) => ts.to_string(),
    }
}

/// Meteo warnings for this point's powiat `teryt`, one per warning that covers it,
/// each tagged with its severity level (1/2/3) for colour grouping. `today` (in the
/// timestamps' timezone) drives the relative "today/yesterday/tomorrow" formatting.
pub fn meteo_warnings(warn_json: &Value, teryt: &str, today: NaiveDate) -> Vec<Warning> {
    let arr = match warn_json.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter(|w| {
            w.get("teryt")
                .and_then(Value::as_array)
                .map(|codes| codes.iter().any(|c| c.as_str() == Some(teryt)))
                .unwrap_or(false)
        })
        .filter_map(|w| {
            let stopien = w.get("stopien").and_then(Value::as_str)?;
            let level = parse_level(stopien)?;
            let name = w.get("nazwa_zdarzenia").and_then(Value::as_str).unwrap_or("");
            let from = drop_seconds(w.get("obowiazuje_od").and_then(Value::as_str).unwrap_or(""));
            let until = drop_seconds(w.get("obowiazuje_do").and_then(Value::as_str).unwrap_or(""));
            let (from_disp, until_disp) = (humanize_ts(from, today), humanize_ts(until, today));
            Some(Warning {
                level,
                from: from.to_string(),
                until: until.to_string(),
                line: format!("{name} — from {from_disp} until {until_disp}"),
            })
        })
        .collect()
}

/// Regular hydrological warnings (levels 1/2/3) for this point's river basin `kod`.
/// The hardcoded drought level (-1, susza hydrologiczna) is excluded — see
/// [`drought_hits_basin`] for that separate notice.
pub fn hydro_warnings(hydro_json: &Value, kod: &str, today: NaiveDate) -> Vec<Warning> {
    let arr = match hydro_json.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|w| {
            let stopien = w.get("stopień").and_then(Value::as_str)?;
            let level = parse_level(stopien)?;
            if level < 1 {
                return None; // drought (-1) is handled as a separate notice
            }
            if !hydro_covers_basin(w, kod) {
                return None;
            }
            let name = w.get("zdarzenie").and_then(Value::as_str).unwrap_or("");
            let from = drop_seconds(w.get("data_od").and_then(Value::as_str).unwrap_or(""));
            let until = drop_seconds(w.get("data_do").and_then(Value::as_str).unwrap_or(""));
            let (from_disp, until_disp) = (humanize_ts(from, today), humanize_ts(until, today));
            Some(Warning {
                level,
                from: from.to_string(),
                until: until.to_string(),
                line: format!("{name} — from {from_disp} until {until_disp}"),
            })
        })
        .collect()
}

/// Whether a hydro warning's affected areas include the basin `kod`.
fn hydro_covers_basin(w: &Value, kod: &str) -> bool {
    w.get("obszary")
        .and_then(Value::as_array)
        .map(|areas| {
            areas.iter().any(|a| {
                a.get("kod_zlewni")
                    .and_then(Value::as_array)
                    .map(|codes| codes.iter().any(|c| c.as_str() == Some(kod)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Group warnings into boxes by severity, highest first: returns `(level, lines)` for
/// each of levels 3, 2, 1 that has any warnings. Within a box, lines are ordered
/// closest-first: earliest start, then earliest end (the ISO-like timestamps sort
/// chronologically as plain strings).
pub fn boxes_by_level(warnings: &[Warning]) -> Vec<(i64, Vec<String>)> {
    let mut out = Vec::new();
    for level in [3, 2, 1] {
        let mut group: Vec<&Warning> = warnings.iter().filter(|w| w.level == level).collect();
        group.sort_by(|a, b| (&a.from, &a.until).cmp(&(&b.from, &b.until)));
        if !group.is_empty() {
            out.push((level, group.iter().map(|w| w.line.clone()).collect()));
        }
    }
    out
}

/// A river basin, identified by its KOD and human NAZWA.
pub struct Basin {
    pub kod: String,
    pub nazwa: String,
}

/// Locate the river basin containing (lat, lon) by point-in-polygon over zlew.json.
/// Returns the first matching feature's (KOD, NAZWA).
pub fn find_basin(zlew_path: &Path, lat: f64, lon: f64) -> Option<Basin> {
    let text = std::fs::read_to_string(zlew_path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let features = v.get("features").and_then(Value::as_array)?;
    for f in features {
        let geom = f.get("geometry")?;
        let gtype = geom.get("type").and_then(Value::as_str).unwrap_or("");
        let coords = geom.get("coordinates")?;
        // Normalise to a list of polygons, each a list of rings (exterior first).
        let polys: Vec<&Value> = match gtype {
            "Polygon" => vec![coords],
            "MultiPolygon" => coords.as_array().map(|a| a.iter().collect()).unwrap_or_default(),
            _ => continue,
        };
        let hit = polys.iter().any(|poly| {
            poly.get(0)
                .and_then(ring_to_vec)
                .map(|ring| point_in_ring(&ring, lon, lat))
                .unwrap_or(false)
        });
        if hit {
            let props = f.get("properties");
            let kod = props.and_then(|p| p.get("KOD")).and_then(Value::as_str)?.to_string();
            let nazwa = props
                .and_then(|p| p.get("NAZWA"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            return Some(Basin { kod, nazwa });
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

/// True if a hydrological drought (`stopień == "-1"` or a "susza" event) is active for
/// the basin `kod`, from a `warningshydro` body.
pub fn drought_hits_basin(hydro_json: &Value, kod: &str) -> bool {
    let arr = match hydro_json.as_array() {
        Some(a) => a,
        None => return false,
    };
    arr.iter().any(|w| {
        let is_drought = w.get("stopień").and_then(Value::as_str) == Some("-1")
            || w.get("zdarzenie")
                .and_then(Value::as_str)
                .map(|s| s.to_lowercase().contains("susza"))
                .unwrap_or(false);
        is_drought && hydro_covers_basin(w, kod)
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
    fn meteo_warnings_filter_by_teryt_and_carry_level() {
        let warn: Value = serde_json::from_str(
            r#"[
              {"nazwa_zdarzenia":"Upał","stopien":"3","obowiazuje_od":"2026-08-04 12:00:00","obowiazuje_do":"2026-08-01 20:00:00","teryt":["1206","1201"]},
              {"nazwa_zdarzenia":"Upał","stopien":"2","obowiazuje_od":"2026-08-03 12:00:00","obowiazuje_do":"2026-08-01 20:00:00","teryt":["1206"]},
              {"nazwa_zdarzenia":"Burze","stopien":"1","obowiazuje_od":"2026-07-31 15:00:00","obowiazuje_do":"2026-07-31 21:00:00","teryt":["1465"]}
            ]"#,
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let ws = meteo_warnings(&warn, "1206", today);
        assert_eq!(ws.len(), 2);
        // from is tomorrow (08-04); until is 08-01, two days back -> shown absolute
        assert!(ws.contains(&Warning {
            level: 3,
            from: "2026-08-04 12:00".into(),
            until: "2026-08-01 20:00".into(),
            line: "Upał — from tomorrow 12:00 until 2026-08-01 20:00".into()
        }));
        // from is today (08-03) -> time only
        assert!(ws.contains(&Warning {
            level: 2,
            from: "2026-08-03 12:00".into(),
            until: "2026-08-01 20:00".into(),
            line: "Upał — from 12:00 until 2026-08-01 20:00".into()
        }));
        assert!(meteo_warnings(&warn, "9999", today).is_empty());
    }

    fn w(level: i64, from: &str, until: &str, line: &str) -> Warning {
        Warning {
            level,
            from: from.into(),
            until: until.into(),
            line: line.into(),
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
        assert_eq!(humanize_ts("2026-08-02 23:59", today), "yesterday 23:59");
        assert_eq!(humanize_ts("2026-08-04 06:00", today), "tomorrow 06:00");
        assert_eq!(humanize_ts("2026-08-06 20:00", today), "2026-08-06 20:00"); // further out
        assert_eq!(humanize_ts("2026-07-31 12:00", today), "2026-07-31 12:00"); // 3 days back
        assert_eq!(humanize_ts("", today), ""); // non-conforming
    }

    #[test]
    fn boxes_by_level_groups_highest_first() {
        let ws = vec![
            w(1, "2026-01-01 00:00:00", "2026-01-02 00:00:00", "a"),
            w(3, "2026-01-01 00:00:00", "2026-01-02 00:00:00", "b"),
            w(1, "2026-01-03 00:00:00", "2026-01-04 00:00:00", "c"),
        ];
        let boxes = boxes_by_level(&ws);
        assert_eq!(
            boxes,
            vec![
                (3, vec!["b".to_string()]),
                (1, vec!["a".to_string(), "c".to_string()]),
            ]
        );
    }

    #[test]
    fn boxes_by_level_sorts_lines_closest_first() {
        // Same level, given out of order; expect earliest `from` first, then earliest
        // `until` as the tie-breaker.
        let ws = vec![
            w(2, "2026-08-05 00:00:00", "2026-08-06 00:00:00", "later-start"),
            w(2, "2026-08-03 00:00:00", "2026-08-09 00:00:00", "early-start-late-end"),
            w(2, "2026-08-03 00:00:00", "2026-08-04 00:00:00", "early-start-early-end"),
        ];
        let boxes = boxes_by_level(&ws);
        assert_eq!(
            boxes,
            vec![(
                2,
                vec![
                    "early-start-early-end".to_string(),
                    "early-start-late-end".to_string(),
                    "later-start".to_string(),
                ]
            )]
        );
    }

    #[test]
    fn hydro_warnings_filter_by_basin_and_exclude_drought() {
        let hydro: Value = serde_json::from_str(
            r#"[
              {"stopień":"-1","zdarzenie":"Susza hydrologiczna","data_do":"2026-09-01 00:00:00",
               "obszary":[{"kod_zlewni":["R_K_MP_1"]}]},
              {"stopień":"2","zdarzenie":"Gwałtowne wzrosty stanów wody","data_od":"2026-08-03 14:10:00","data_do":"2026-08-03 22:00:00",
               "obszary":[{"kod_zlewni":["R_K_MP_1","R_K_MP_9"]}]},
              {"stopień":"1","zdarzenie":"Wezbranie","data_od":"2026-08-03 18:00:00","data_do":"2026-08-04 06:00:00",
               "obszary":[{"kod_zlewni":["R_K_MP_2"]}]}
            ]"#,
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let ws = hydro_warnings(&hydro, "R_K_MP_1", today);
        // only the level-2 warning covers R_K_MP_1; drought (-1) is excluded here.
        // both timestamps are today -> shown as time only
        assert_eq!(
            ws,
            vec![Warning {
                level: 2,
                from: "2026-08-03 14:10".into(),
                until: "2026-08-03 22:00".into(),
                line: "Gwałtowne wzrosty stanów wody — from 14:10 until 22:00".into()
            }]
        );
    }

    #[test]
    fn drought_matches_basin_by_stopien_or_event() {
        let hydro: Value = serde_json::from_str(
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
