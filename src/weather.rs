//! Pure weather-text helpers: a bearing-to-compass converter and a sky-condition
//! describer. No I/O. Display words come from the `text` module.

use crate::text::{self, PrecipKind};

/// Bearing (degrees) -> 16-point compass point (N, NNE, NE, …), rounded to the
/// nearest 22.5°. Empty/non-numeric input -> "". Negative bearings wrap into range.
pub fn cardinal(deg: &str) -> String {
    let d: f64 = match deg.trim().parse() {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    const C: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    // `as i64` truncates toward zero; the modulo below folds negatives into range.
    let idx = (d / 22.5 + 0.5) as i64;
    let idx = ((idx % 16) + 16) % 16;
    C[idx as usize].to_string()
}

/// Human-readable sky condition from an Icon10 code (n<cloud><precip><d|n>), refined
/// with the 10-min rain/snow/precip amounts (mm) for precipitation type + intensity.
pub fn condition(icon: &str, rain: f64, snow: f64, prec: f64) -> String {
    let chars: Vec<char> = icon.chars().collect();
    let cloud = chars.get(1).copied();
    let dn = chars.last().copied();
    let night = dn == Some('n');

    let mut sky: String = match cloud {
        Some('0') => (if night {
            text::SKY_CLEAR_NIGHT
        } else {
            text::SKY_CLEAR_DAY
        })
        .into(),
        Some('1') | Some('2') => text::SKY_FEW.into(),
        Some('3') | Some('4') => text::SKY_SCATTERED.into(),
        Some('5') | Some('6') | Some('7') => text::SKY_BROKEN.into(),
        Some('8') => text::SKY_OVERCAST.into(),
        _ => {
            if icon.is_empty() {
                String::new()
            } else {
                text::SKY_UNKNOWN.into()
            }
        }
    };

    // Precipitation segment of the icon code, e.g. z00 (dry) / z60 (rain).
    let seg: String = if chars.len() >= 2 {
        chars[2..chars.len() - 1].iter().collect()
    } else {
        String::new()
    };

    if (!seg.is_empty() && seg != "z00") || rain > 0.0 || snow > 0.0 || prec > 0.0 {
        let mut mm = if prec > rain { prec } else { rain };
        let kind = if snow > 0.0 {
            PrecipKind::Snow
        } else if rain > 0.0 || mm > 0.0 {
            PrecipKind::Rain
        } else {
            PrecipKind::Mixed
        };
        if snow > mm {
            mm = snow;
        }
        let phrase = text::precip_phrase(kind, mm);
        if sky.is_empty() {
            sky = phrase;
        } else {
            sky = format!("{sky}, {phrase}");
        }
    }
    sky
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_points() {
        assert_eq!(cardinal("0"), "N");
        assert_eq!(cardinal("45"), "NE");
        assert_eq!(cardinal("90"), "E");
        assert_eq!(cardinal("180"), "S");
        assert_eq!(cardinal("270"), "W");
    }

    #[test]
    fn cardinal_rounding_and_wrap() {
        assert_eq!(cardinal("350"), "N");
        assert_eq!(cardinal("360"), "N");
        assert_eq!(cardinal("22.5"), "NNE");
        assert_eq!(cardinal("-10"), "N");
    }

    #[test]
    fn cardinal_empty_or_nonnumeric() {
        assert_eq!(cardinal(""), "");
        assert_eq!(cardinal("abc"), "");
    }

    #[test]
    fn condition_sky_tiers() {
        assert_eq!(condition("n0z00d", 0.0, 0.0, 0.0), "słonecznie");
        assert_eq!(condition("n0z00n", 0.0, 0.0, 0.0), "bezchmurnie");
        assert_eq!(condition("n1z00d", 0.0, 0.0, 0.0), "zachmurzenie małe");
        assert_eq!(
            condition("n3z00d", 0.0, 0.0, 0.0),
            "zachmurzenie umiarkowane"
        );
        assert_eq!(condition("n5z00d", 0.0, 0.0, 0.0), "zachmurzenie duże");
        assert_eq!(condition("n8z00d", 0.0, 0.0, 0.0), "zachmurzenie całkowite");
    }

    #[test]
    fn condition_with_precip() {
        assert_eq!(
            condition("n7z60d", 0.5, 0.0, 0.5),
            "zachmurzenie duże, umiarkowany deszcz"
        );
        assert_eq!(
            condition("n8z00d", 0.0, 2.0, 2.0),
            "zachmurzenie całkowite, silny śnieg"
        );
        assert_eq!(
            condition("n1z00d", 0.1, 0.0, 0.1),
            "zachmurzenie małe, słaby deszcz"
        );
    }

    #[test]
    fn condition_empty() {
        assert_eq!(condition("", 0.0, 0.0, 0.0), "");
    }
}
