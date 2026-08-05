# pogoda

A terminal point-weather nowcast from **IMGW-PIB**, the Polish meteorological
service. Give it a place; it prints the current conditions for that exact point,
plus any active meteorological and hydrological (drought) warnings there. The UI
is **Polish** (IMGW is a Poland-only service).

```
❯ pogoda -p "Warszawa,PL"
┌─ OSTRZEŻENIE! (stopień 3, 90%) ─────────────────────────────────────────┐
│ Upał — od 2026-08-03 20:00 do jutra 20:00                               │
│ Prognozuje się upały. Temperatura maksymalna w dzień od 33°C do 38°C.   │
└─────────────────────────────────────────────────────────────────────────┘
┌─ UWAGA ────────────────────────────────────────────────────────┐
│ Susza hydrologiczna — zlewnia Wisła od Dęblina do ujścia Narwi │
└────────────────────────────────────────────────────────────────┘
 Pogoda: Warszawa — 29.1 °C, słonecznie  (IMGW HYBRID, odczuwalna 30.7 °C)
   Opady: brak (sucho przez najbliższe 63h)
   Wiatr: 1.8 m/s SSE (152°), w porywach 7.4 m/s
   Ciśnienie: 1013 hPa
   Wilgotność: 57%
   Zachmurzenie: 0%
   Wschód: 05:02   Zachód: 20:21   (dzień 15h 19m)

Dane pochodzą z https://meteo.imgw.pl/
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
cargo build --release   # binary at target/release/pogoda
```

## Usage

```
pogoda ["City,CC" | "lat,lon"]
```

- `pogoda` — forecast for `weather_place` from the config file.
- `pogoda Kraków` / `pogoda "Warszawa,PL"` — forecast for a named place.
- `pogoda "50.06,19.94"` — forecast for coordinates.

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
cargo test     # unit + integration tests
cargo build
```

Tests come in two layers:

- **Unit tests** (in each module) cover the pure logic: place parsing, forecast
  parsing, warning filtering, point-in-polygon, box drawing, and the retry/backoff
  schedule.
- **Integration tests** (`tests/`) run against a local mock HTTP server
  ([mockito](https://crates.io/crates/mockito)). `tests/retry.rs` verifies the
  retry policy against real HTTP statuses (404/422/5xx are retried; a later 200
  recovers; it gives up after the cap). `tests/pipeline.rs` drives the whole
  `run_place` flow — geocode → token → forecast → warnings → render — with every
  endpoint mocked, asserting on the rendered output.

Endpoints are injectable (`Client::with(Endpoints { .. }, ..)`), which is how the
integration tests point every call at the mock server.
