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

/// Build the meteo-warning lines for this point's powiat `teryt`, one per warning that
/// covers it.
pub fn meteo_lines(warn_json: &Value, teryt: &str) -> Vec<String> {
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
        .map(|w| {
            let name = w.get("nazwa_zdarzenia").and_then(Value::as_str).unwrap_or("");
            let stopien = w.get("stopien").and_then(Value::as_str).unwrap_or("");
            let until = w.get("obowiazuje_do").and_then(Value::as_str).unwrap_or("");
            format!("{name} — level {stopien}, until {until}")
        })
        .collect()
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
    for w in arr {
        let is_drought = w.get("stopień").and_then(Value::as_str) == Some("-1")
            || w.get("zdarzenie")
                .and_then(Value::as_str)
                .map(|s| s.to_lowercase().contains("susza"))
                .unwrap_or(false);
        if !is_drought {
            continue;
        }
        if let Some(areas) = w.get("obszary").and_then(Value::as_array) {
            for a in areas {
                if let Some(codes) = a.get("kod_zlewni").and_then(Value::as_array) {
                    if codes.iter().any(|c| c.as_str() == Some(kod)) {
                        return true;
                    }
                }
            }
        }
    }
    false
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
    fn meteo_lines_filter_by_teryt() {
        let warn: Value = serde_json::from_str(
            r#"[
              {"nazwa_zdarzenia":"Upał","stopien":"3","obowiazuje_do":"2026-08-01 20:00:00","teryt":["1206","1201"]},
              {"nazwa_zdarzenia":"Burze","stopien":"1","obowiazuje_do":"2026-07-31 21:00:00","teryt":["1465"]}
            ]"#,
        )
        .unwrap();
        let lines = meteo_lines(&warn, "1206");
        assert_eq!(lines, vec!["Upał — level 3, until 2026-08-01 20:00:00"]);
        assert!(meteo_lines(&warn, "9999").is_empty());
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
