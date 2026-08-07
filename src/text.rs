//! All user-facing text, in one place. The UI is Polish: IMGW is a Poland-only
//! service, so the forecast, warnings, errors and help are written in Polish rather
//! than translating the (Polish) source data into English. Keeping every display
//! string here — instead of scattered inline literals — makes the wording reviewable
//! and consistent. Compass letters (N/NE/…) and the product name "IMGW HYBRID" are
//! left as-is.

// --- sky conditions (weather::condition) — IMGW-style cloud scale ---
pub const SKY_CLEAR_NIGHT: &str = "bezchmurnie";
pub const SKY_CLEAR_DAY: &str = "słonecznie";
pub const SKY_FEW: &str = "zachmurzenie małe";
pub const SKY_SCATTERED: &str = "zachmurzenie umiarkowane";
pub const SKY_BROKEN: &str = "zachmurzenie duże";
pub const SKY_OVERCAST: &str = "zachmurzenie całkowite";
pub const SKY_UNKNOWN: &str = "nieznane";

/// Precipitation type, for the intensity phrase's gender agreement.
pub enum PrecipKind {
    Rain,
    Snow,
    Mixed,
}

/// "<intensity> <precip>" with the adjective agreeing in gender: masculine for
/// deszcz/śnieg (słaby/umiarkowany/silny), plural for opady (słabe/umiarkowane/silne).
pub fn precip_phrase(kind: PrecipKind, mm: f64) -> String {
    let (weak, moderate, strong, noun) = match kind {
        PrecipKind::Rain => ("słaby", "umiarkowany", "silny", "deszcz"),
        PrecipKind::Snow => ("słaby", "umiarkowany", "silny", "śnieg"),
        PrecipKind::Mixed => ("słabe", "umiarkowane", "silne", "opady"),
    };
    let adj = if mm < 0.3 {
        weak
    } else if mm < 1.5 {
        moderate
    } else {
        strong
    };
    format!("{adj} {noun}")
}

// --- forecast block (lib::print_forecast) ---

/// Precipitation-type labels for the precip line.
pub const PRECIP_RAIN: &str = "Deszcz";
pub const PRECIP_SNOW: &str = "Śnieg";
pub const PRECIP_RAIN_AND_SNOW: &str = "Deszcz i śnieg";
pub const PRECIP_MIXED: &str = "Opady";

/// Banner: place kept in the nominative (Polish place-name declension is irregular and
/// not worth a dependency), so "Pogoda: <place> — …" rather than "Pogoda w …".
pub fn banner(place: &str, temp: f64, cond_suffix: &str, feels: f64) -> String {
    format!(" Pogoda: {place} — {temp:.1} °C{cond_suffix}  (IMGW HYBRID, odczuwalna {feels:.1} °C)")
}

/// Rain-intensity label for the current rate in mm/h — the "raining now" prefix,
/// including its own punctuation (e.g. `Mżawka.`, `Deszcz: silny.`, `Ulewa!!!`).
/// Thresholds: <0.25 drizzle, ≤2 light, ≤5 moderate, ≤10 heavy, ≤15 very heavy,
/// ≤30 downpour, else torrential.
pub fn rain_intensity(mmh: f64) -> &'static str {
    if mmh < 0.25 {
        "Mżawka."
    } else if mmh <= 2.0 {
        "Deszcz: lekki."
    } else if mmh <= 5.0 {
        "Deszcz: umiarkowany."
    } else if mmh <= 10.0 {
        "Deszcz: silny."
    } else if mmh <= 15.0 {
        "Deszcz: bardzo silny!"
    } else if mmh <= 30.0 {
        "Ulewa!!!"
    } else {
        "Nawałnica!!!"
    }
}

/// Raining now: `prefix` (a label like "Deszcz:"/"Śnieg:" or an intensity like
/// "Ulewa!!!", punctuation included) then the amount over the next `rain_h` hours, then
/// (if the rain stops before the window ends) dry for `tail_h`. At most two clauses — a
/// reader doesn't need every individual shower.
pub fn rain_then_dry(prefix: &str, amount: f64, rain_h: i64, tail_h: Option<i64>) -> String {
    let mut s = format!("   {prefix} {amount:.1} mm w ciągu najbliższych {rain_h}h");
    if let Some(t) = tail_h {
        s.push_str(&format!(", potem sucho przez kolejne {t}h"));
    }
    s
}

/// Dry now, rain later: dry for `lead_h` hours, then the amount over the rest of the
/// window (`rain_h`). "sucho przez najbliższe Xh, potem M mm w ciągu kolejnych Yh".
pub fn dry_then_rain(kind: &str, lead_h: i64, amount: f64, rain_h: i64) -> String {
    format!(
        "   {kind}: sucho przez najbliższe {lead_h}h, potem {amount:.1} mm w ciągu kolejnych {rain_h}h"
    )
}

/// No precipitation anywhere in the window.
pub fn precip_none(hours: i64) -> String {
    format!("   Opady: sucho przez najbliższe {hours}h")
}

/// Wind-direction phrases in the genitive, ready to follow "Wiatr " (→ "Wiatr z
/// zachodu"). Indexed by the 8-point compass: N, NE, E, SE, S, SW, W, NW. "ze wschodu"
/// takes the euphonic "ze".
pub const WIND_FROM: [&str; 8] = [
    "z północy",
    "z północnego wschodu",
    "ze wschodu",
    "z południowego wschodu",
    "z południa",
    "z południowego zachodu",
    "z zachodu",
    "z północnego zachodu",
];

/// Wind line: "Wiatr <z kierunku>: <speed> m/s (<deg>°)[, w porywach <gust> m/s]".
/// `dir_phrase` is a `WIND_FROM` entry (empty if the bearing is unknown); `deg` is the
/// raw bearing (empty to omit).
pub fn wind_line(speed: &str, dir_phrase: &str, deg: &str, gust: Option<&str>) -> String {
    let mut s = String::from("   Wiatr");
    if !dir_phrase.is_empty() {
        s.push_str(&format!(" {dir_phrase}"));
    }
    s.push_str(&format!(": {speed} m/s"));
    if !deg.is_empty() {
        s.push_str(&format!(" ({deg}°)"));
    }
    if let Some(g) = gust {
        s.push_str(&format!(", w porywach {g} m/s"));
    }
    s
}

pub fn pressure_line(hpa: f64) -> String {
    format!("   Ciśnienie: {hpa:.0} hPa")
}

pub fn humidity_line(pct: &str) -> String {
    format!("   Wilgotność: {pct}%")
}

pub fn cloud_line(pct: &str) -> String {
    format!("   Zachmurzenie: {pct}%")
}

pub fn sun_line(sunrise: &str, sunset: &str, hours: i64, mins: i64) -> String {
    format!("   Wschód: {sunrise}   Zachód: {sunset}   (dzień {hours}h {mins:02}m)")
}

/// Source-attribution trailer printed under every forecast. Credits IMGW-PIB as the
/// data source (the tool re-presents their public data; it is not their product).
pub const SOURCE: &str = "Źródło danych: Instytut Meteorologii i Gospodarki Wodnej – PIB";

// --- warnings ---

/// Box caption carrying the severity level and (if present) probability.
pub fn warning_caption(level: i64, prob: &str) -> String {
    if prob.is_empty() {
        format!("OSTRZEŻENIE! (stopień {level})")
    } else {
        format!("OSTRZEŻENIE! (stopień {level}, {prob}%)")
    }
}

/// Warning headline: "<event> — od <start> do <end>".
pub fn warning_headline(event: &str, from: &str, until: &str) -> String {
    format!("{event} — od {from} do {until}")
}

/// Caption for the (quiet) drought notice box.
pub const NOTICE_CAPTION: &str = "UWAGA!";

/// Drought notice body for a river basin.
pub fn drought_notice(basin: &str) -> String {
    format!("Susza hydrologiczna — zlewnia {basin}")
}

/// Relative-day prefixes for timestamps on the adjacent days. These always appear
/// inside "od … do …" (see `warning_headline`), so "tomorrow" is the genitive "jutra"
/// ("od jutra"/"do jutra"); "wczoraj" is an indeclinable adverb and stays as-is.
pub const YESTERDAY: &str = "wczoraj";
pub const TOMORROW: &str = "jutra";

// --- errors & help ---

/// Program name, used as the stderr error prefix.
pub const PROGRAM: &str = "pogoda";

pub const GEOCODE_FAILED: &str = "błąd zapytania geokodowania";

pub fn not_geocoded(town: &str, country: Option<&str>) -> String {
    match country {
        Some(cc) => format!("nie udało się zlokalizować \"{town}\" w kraju {cc}"),
        None => format!("nie udało się zlokalizować \"{town}\""),
    }
}

pub const FORECAST_FAILED: &str = "nie udało się pobrać prognozy IMGW";

pub fn no_forecast_data(label: &str, lat: f64, lon: f64) -> String {
    format!("brak danych prognozy dla \"{label}\" ({lat}, {lon})")
}

pub const ERR_NO_LOCATION: &str =
    "Błąd: nie podano lokalizacji, a `weather_place` nie jest ustawione";
pub const USAGE: &str = "Użycie: pogoda [\"Miasto,KK\" | \"szer,dług\"]";
pub const EXAMPLE: &str = "Przykład: pogoda \"Warszawa,PL\"";

pub fn err_unknown_option(opt: &str) -> String {
    format!("Błąd: nieznana opcja `{opt}`")
}

/// Shown for `weather_place` in help when the config key is absent.
pub const UNSET: &str = "nie ustawione";

pub const HELP_DESC: &str =
    "  Prognoza punktowa IMGW dla lokalizacji, wraz z aktywnymi ostrzeżeniami.";
pub const HELP_EXAMPLES: &str = "  Przykłady: pogoda \"Warszawa,PL\"   |   pogoda \"52.24,21.03\"";

pub fn help_default(current: &str) -> String {
    format!("  Bez argumentu używa `weather_place` z pliku konfiguracyjnego (obecnie: {current}).")
}

pub fn help_config(path: &str) -> String {
    format!("  Konfiguracja: {path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_now_ending_before_window_end() {
        assert_eq!(
            rain_then_dry("Deszcz:", 1.6, 6, Some(58)),
            "   Deszcz: 1.6 mm w ciągu najbliższych 6h, potem sucho przez kolejne 58h"
        );
    }

    #[test]
    fn rain_now_through_whole_window() {
        assert_eq!(
            rain_then_dry("Deszcz:", 1.6, 64, None),
            "   Deszcz: 1.6 mm w ciągu najbliższych 64h"
        );
    }

    #[test]
    fn rain_now_with_intensity_prefix() {
        // the prefix carries its own punctuation and replaces the plain "Deszcz:"
        assert_eq!(
            rain_then_dry(rain_intensity(11.0), 14.0, 3, Some(21)),
            "   Deszcz: bardzo silny! 14.0 mm w ciągu najbliższych 3h, potem sucho przez kolejne 21h"
        );
        assert_eq!(
            rain_then_dry(rain_intensity(0.2), 3.2, 3, Some(21)),
            "   Mżawka. 3.2 mm w ciągu najbliższych 3h, potem sucho przez kolejne 21h"
        );
    }

    #[test]
    fn rain_intensity_thresholds() {
        assert_eq!(rain_intensity(0.24), "Mżawka.");
        assert_eq!(rain_intensity(0.25), "Deszcz: lekki.");
        assert_eq!(rain_intensity(2.0), "Deszcz: lekki.");
        assert_eq!(rain_intensity(2.1), "Deszcz: umiarkowany.");
        assert_eq!(rain_intensity(5.0), "Deszcz: umiarkowany.");
        assert_eq!(rain_intensity(9.0), "Deszcz: silny.");
        assert_eq!(rain_intensity(11.0), "Deszcz: bardzo silny!");
        assert_eq!(rain_intensity(20.0), "Ulewa!!!");
        assert_eq!(rain_intensity(40.0), "Nawałnica!!!");
    }

    #[test]
    fn dry_then_rain_over_the_rest() {
        assert_eq!(
            dry_then_rain("Śnieg", 16, 5.3, 48),
            "   Śnieg: sucho przez najbliższe 16h, potem 5.3 mm w ciągu kolejnych 48h"
        );
    }
}
