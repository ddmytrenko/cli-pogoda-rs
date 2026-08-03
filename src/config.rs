//! Flat `key = value` INI config, read from `$XDG_CONFIG_HOME/imgw-rs/config.ini`
//! (falling back to `~/.config/...`). Comments (`#`/`;`), blank lines and `[section]`
//! headers are ignored; values keep their case, keys are lowercased. The only key the
//! app reads today is `weather_place`, but any key is retained so the file can grow
//! without code changes.

use std::collections::HashMap;
use std::path::PathBuf;

pub struct Config {
    map: HashMap<String, String>,
}

impl Config {
    /// Load config, returning an empty config if the file is absent or unreadable.
    pub fn load() -> Self {
        let mut map = HashMap::new();
        if let Some(path) = Self::path() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                Self::parse_into(&text, &mut map);
            }
        }
        Config { map }
    }

    fn parse_into(text: &str, map: &mut HashMap<String, String>) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with(';')
                || line.starts_with('[')
            {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim().to_lowercase();
                let mut val = v.trim();
                // strip a single pair of surrounding quotes, if present
                if val.len() >= 2
                    && ((val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\'')))
                {
                    val = &val[1..val.len() - 1];
                }
                if !key.is_empty() {
                    map.insert(key, val.to_string());
                }
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    /// `$XDG_CONFIG_HOME/imgw-rs/config.ini`, else `~/.config/imgw-rs/config.ini`.
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("config.ini"))
    }
}

/// The app's config dir: `$XDG_CONFIG_HOME/imgw-rs` or `~/.config/imgw-rs`.
pub fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// The app's cache dir: `$XDG_CACHE_HOME/imgw-rs` or `~/.cache/imgw-rs`.
pub fn cache_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CACHE_HOME", ".cache")
}

fn xdg_dir(env: &str, fallback: &str) -> Option<PathBuf> {
    let base = match std::env::var_os(env) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(fallback),
    };
    Some(base.join("imgw-rs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_ignoring_comments_and_sections() {
        let mut m = HashMap::new();
        Config::parse_into(
            "# a comment\n[section]\nweather_place = Warsaw,PL\n; semi comment\nempty=\n",
            &mut m,
        );
        assert_eq!(
            m.get("weather_place").map(String::as_str),
            Some("Warsaw,PL")
        );
        assert_eq!(m.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn strips_surrounding_quotes_and_lowercases_key() {
        let mut m = HashMap::new();
        Config::parse_into("Weather_Place = \"52.24, 21.03\"\n", &mut m);
        assert_eq!(
            m.get("weather_place").map(String::as_str),
            Some("52.24, 21.03")
        );
    }
}
