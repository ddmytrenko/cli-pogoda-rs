# imgw

A terminal point-weather nowcast from **IMGW-PIB**, the Polish meteorological
service. Give it a place; it prints the current conditions for that exact point,
plus any active meteorological and hydrological (drought) warnings there.

```
❯ imgw
┌─ WARNING! ────────────────────────────────┐
│ Upał — level 3, until 2026-08-01 20:00:00 │
└───────────────────────────────────────────┘
┌─ NOTICE ──────────────────────────────────────────────────┐
│ Susza hydrologiczna (hydrological drought) — Rudawa basin │
└───────────────────────────────────────────────────────────┘
 Weather in Krzeszowice: 32.7 °C, sunny  (IMGW HYBRID, feels 32.6 °C)
   Precipitation: none (dry next 61h)
   Wind: 4.3 m/s SW (230°), gust 8.1 m/s
   Pressure: 1015 hPa
   Humidity: 32%
   Cloud: 0%
   Sunrise: 05:08   Sunset: 20:27   (day 15h 19m)
```

## How it works

IMGW's HYBRID nowcast endpoint serves a forecast for an exact lat/lon (not a
fixed station), so a village reports its own point. IMGW has no geocoder, so a
place *name* is first geocoded via Open-Meteo; bare coordinates are used directly
(with Open-Meteo for the timezone and BigDataCloud for a display name).

Warnings are filtered to the point, not the country:

- **Meteo** warnings come from the nationwide feed, filtered to the point's
  *powiat* (county) TERYT code.
- **Hydrological drought** is filtered to the point's *river basin*, found by
  point-in-polygon over IMGW's basin polygons (cached locally).

## Install

```sh
cargo install --path .
# or
cargo build --release   # binary at target/release/imgw
```

## Usage

```
imgw [-p|--place "City,CC"|"lat,lon"] [location]
```

- `imgw` — forecast for `weather_place` from the config file.
- `imgw Krakow` / `imgw "Warsaw,PL"` — forecast for a named place.
- `imgw "50.06,19.94"` — forecast for coordinates.
- `imgw -p "Gdansk"` — the `-p/--place` flag wins over a positional argument.

Colour output disables itself when stdout is not a terminal or `NO_COLOR` is set.

## Configuration

Config lives at `$XDG_CONFIG_HOME/imgw-rs/config.ini` (usually
`~/.config/imgw-rs/config.ini`). See [`config.example.ini`](config.example.ini).

| Key             | Meaning                                            |
| --------------- | -------------------------------------------------- |
| `weather_place` | Default location (`City,CC` or `lat,lon`) when run with no argument. |

Cached data (API token, basin polygons) lives under
`$XDG_CACHE_HOME/imgw-rs/`.

## Development

```sh
cargo test     # unit tests (pure logic: geocoding parse, forecast parse, warnings, box drawing)
cargo build
```
