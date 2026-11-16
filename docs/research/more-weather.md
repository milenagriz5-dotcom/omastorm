# More Weather: what Omastorm should take

Research note, 2026-11-15. Compares More Weather 3.5.0 ([T-The-Coder/more-weather](https://github.com/T-The-Coder/more-weather) at `201f44d`, MIT, "Copyright (c) 2026 T-The-Coder", `LICENSE:1-5`) with Omastorm on `feat/inpe-brazil`. More Weather is a pure QML/JS bar plugin whose radar view is styled after Omastorm (`README.md:500`, `docs/radar-rebuild.md:3-6`). The clone was read only; none of its code or scripts was run. Paths below are relative to the clone (`more-weather/…`) or to the Omastorm root.

## Ranked top 5

Scored on value to the INPE/Brazil work first, then general value.

| # | Item | Verdict | Effort | Where |
| --- | --- | --- | --- | --- |
| 1 | Say what the INPE values are: dBZ reconstructed from INPE's colour legend, stamped on a slot, not an observation time | TAKE (idea) | S | `engine/src/inpe.rs` frame metadata, protocol `frame`, UI product line |
| 2 | Age from the newest *contributing observation*, not from the slot stamp (More Weather: "a forecast is never a fresh observation") | TAKE (idea) | S | `engine/src/inpe.rs`, `main.rs` `evidence_age_secs` |
| 3 | INMET Brazil warnings (one keyless JSON for all of Brazil, GeoJSON polygons) | TAKE (idea), after a scope decision | M-L | new engine source, protocol message, UI |
| 4 | Pin the INPE request sequence in a test, so UI or refactor work cannot change what is fetched | TAKE (idea) | S | `engine/src/inpe/tests.rs` |
| 5 | Per-day request accounting and a ceiling for INPE (and a User-Agent) | TAKE (idea) | S-M | `engine/src/inpe.rs`, `live_index.rs` |

Runner-up with the best effort/value ratio outside Brazil: the no-bare-`Color.` lint (item 7 below), S, `scripts/tooling/cli.py`.

## INMET live probe

One plain `curl -sS --max-time 30` GET of `https://apiprevmet3.inmet.gov.br/avisos/ativos`, body capped at 16 MiB, no extra headers, on 2026-11-16 00:35 UTC:

- `curl: (56) Recv failure: Connection reset by peer` after 0.21 s, HTTP code 000, 0 bytes.
- DNS works: the name resolves to `cache20.inmet.gov.br` (201.57.198.188). General HTTPS from the same machine works (`https://example.com/` → 200).
- Per the one-request rule it was not retried. **Liveness, key-free access and the response shape are therefore not confirmed from here.** The reset came before any HTTP response, so it is a server- or network-side refusal (IP or geo filtering, or a filter on the client: More Weather sends its own User-Agent, `WeatherRequest.qml:44`; curl's default was used here). Re-probe from a Brazilian network or with a project User-Agent before building on it.

Shape as More Weather's parser expects it (`Model.js:3063-3113`), unverified:

- Top level: `hoje` (today) and `futuro` (upcoming), each an array of warning rows; either may be missing (`Model.js:3066-3067`).
- Per row: `codigo` or `id`, `descricao` (event), `severidade` (`Perigo Potencial` / `Perigo` / `Grande Perigo`, mapped to moderate/severe/extreme, `Model.js:3070`), `riscos` and `instrucoes` (Python-list-like strings, `['a', 'b']`, `Model.js:3076-3080`), `inicio` / `fim` (Brasília local time without offset; More Weather appends `-03:00`, `Model.js:3072-3075`), `encerrado` (the string `"True"` when closed, `Model.js:3085`), `poligono` (a GeoJSON `Polygon`/`MultiPolygon` geometry, often as a JSON *string*, `Model.js:3087`).
- Size: "All of Brazil in one answer, ~0.4 MB with its warning icons" (`Providers.js:415`); More Weather allows 20 s and 8 MiB (`Providers.js:416`).
- More Weather has no test fixture for this parser (no INMET hit under `tests/`).

## Candidates

### 1. INMET Brazil warnings

**More Weather.** For a place in Brazil the warning chain is INMET first, then the Alert Hub register (`Providers.js:333-334`). The request is the single national JSON (`Providers.js:414-417`). The parser keeps rows whose polygon contains the place, skips rows with no parseable polygon (fail closed, `Model.js:3088`), drops closed rows and duplicates, and caps output at `ALERT_MAX_PER_DOC` = 200 (`Model.js:2385`, `3083`). Text is cut to bounded flat records (`Model.js:2380-2399`). Point-in-polygon handles holes and MultiPolygon (`Model.js:2793-2807`). Links point to `https://alertas2.inmet.gov.br/` (`Model.js:3109`).

**Omastorm.** Nothing found: no warning, alert, CAP or polygon code in `engine/src` or `ui/`. The README says the opposite on purpose: "Color is dBZ, not a rain rate and not a warning" (`README.md:93`) and "These are observations, not forecasts or weather warnings" (`README.md:107`).

**Verdict: TAKE (idea), conditional on a scope decision.** For Brazil this is the most useful thing More Weather has that Omastorm lacks: one keyless request covers the whole INPE envelope, and the polygons can be drawn on the existing map without any per-place lookup. But it contradicts the README's stated scope, so the maintainer has to decide first. If accepted: an engine source that polls every 10-15 minutes with `take_body` capped (1-8 MiB), parses rows into a small typed record (id, event, severity, onset, expires, simplified polygon), and a new protocol message (`warnings`) the UI outlines on the map and lists in the popover. Write the parser fresh in Rust; More Weather's is short and specific, and nothing in it needs copying. Points to get right that More Weather assumes: Brazil has four time zones, and the `-03:00` assumption should be checked against live data; the polygon string-or-object duality; treat a missing polygon as "not here". Effort M for the engine and protocol, L with map drawing. Record INMET's terms the way `docs/inpe-brazil.md` ("Access and license terms") requires for INPE.

### 2. WMO Alert Hub register of official CAP feeds

**More Weather.** One fetch of `https://alert-hub-sources.s3.amazonaws.com/json` (`WeatherAlertLookup.qml:24`, `87`), cached for the session (`WeatherAlertLookup.qml:231-233`). `alertHubFeeds` (`Model.js:3032-3059`) reads `sources[].source` and keeps entries with `authorityCountry` equal to the place's country, `sourceIsOfficial !== false`, `capAlertFeedStatus === "operating"` and a `capAlertFeed` that passes `publicHttpsUrl` (no IP literals, credentials, localhost or `.local`/`.internal` hosts, `Model.js:3008-3018`). It keeps one feed per authority (English if offered), ranks weather services first by a name regex, and takes at most 4 feeds. Each feed (RSS/Atom) lists CAP documents; at most 32 documents are downloaded per refresh, failures included (`Model.js:2389`, `2515-2519`), each capped at 2 MiB (`Model.js:2390`), entries older than 10 days ignored (`WeatherAlertLookup.qml:26`). Documents are cached by URL only while a current feed still references them (`Model.js:2496-2526`). CAP `<polygon>` and `<circle>` areas are tested against the place (`Model.js:2903-2920`).

**Omastorm.** Nothing found.

**Verdict: SKIP for now; TAKE (idea) if warnings are accepted beyond Brazil.** For Brazil, INMET's direct JSON is simpler and is what More Weather itself prefers. The design points worth keeping if Omastorm ever covers OPERA or MRMS regions with warnings: filter on `sourceIsOfficial` and `capAlertFeedStatus`, validate feed URLs before fetching (the register is third-party data), and bound downloads per refresh with failures counted. Effort L (CAP/XML parsing, per-authority quirks).

### 3. Honest labelling of derived values

**More Weather.** Every source is labelled by what it actually is. DWD and JMA images are decoded from colour classes to rain rate, converted with Z = 200 R^1.6, and labelled "Approx. dBZ"; WMS sources it does not decode are labelled "Provider colours"; only RainViewer's encoded bytes get plain "dBZ" (`Panel.qml:1532-1534`, `WeatherRadarLegend.qml:52`, `i18n/en.js:27-28`, `docs/radar-rebuild.md:24-35`, `README.md:78-79`). Unknown colours stay transparent (`RadarStyle.js:77-90`).

**Omastorm.** The INPE adapter goes further than More Weather on rigour: it accepts a layer only if its live legend equals the 74-entry 1 dBZ profile exactly (`engine/src/inpe.rs:108-111`, `328-347`), projects each pixel onto the legend ramp with a 16-unit tolerance (`inpe.rs:283-308`), and rejects a frame with more than 2 % unmatched opaque pixels (`inpe.rs:92-94`, `406-410`). But the output presents itself exactly like a measured product: `product_name: "CAPPI 3 km Reflectivity (composite)"`, `units: "dBZ"`, `measured_code: Some(CODE)` (`inpe.rs:668-686`), and the UI shows `scan.units` under the legend (`ui/RadarWindow.qml:1094`) and the source attribution on the product line (`RadarWindow.qml:677`). `docs/inpe-brazil.md` ("Open before merge", "Precolored source") already flags that this is a reconstruction and that `docs/grid-adapters.md:35` lists precolored tiles as a non-goal.

**Verdict: TAKE (idea).** Not "approximate": Omastorm's inversion is exact to the legend's 1 dBZ step, which More Weather's class decoding is not. The idea to take is that the label states the derivation. Concretely: name the product "CAPPI 3 km reflectivity, from INPE colour legend" (or add a `derived`/`provenance` field to the frame in `docs/protocol.md` that the UI appends to the product line), and keep "dBZ" as the unit. This is the cheapest way to resolve the open precolored-source question honestly, if the maintainer records an exception. Effort S. No code to copy.

### 4. Capping and evenly thinning timelines at 64 frames

**More Weather.** `RADAR_MAX_FRAMES = 64` and `boundedRadarFrames` (`Model.js:3383-3398`): a sorted list longer than 64 is thinned with stride `ceil(n / 64)` counted back from the newest frame, so the newest is kept and the whole window stays covered. Applied to every WMS and catalogue timeline (`Model.js:3438`, `3519`, `3557`). The comment says real catalogues hold 13-40 and the cap guards against a catalogue with thousands of times becoming thousands of image items. `RadarMotion.mjs:19-26` refuses reports over 64 frames outright.

**Omastorm.** Already bounded, by construction rather than thinning. Every polling grid source declares a `HistoryPolicy` (`engine/src/source.rs:20-34`, `390-400`): INPE 12 slots in a 2-hour window (`inpe.rs:70-73`), OPERA 12, MRMS 30. `retain_history` drops the oldest beyond the cap (`main.rs:275-281`) and older backfill beyond a full timeline is refused (`main.rs:1017-1029`). NEXRAD's catalog ring is 60 with a 2-hour TTL (`engine/src/catalog.rs:20-26`). INPE builds its own ten-minute slots (`inpe.rs:829-834`), so a long upstream time list cannot widen the timeline.

**Verdict: SKIP.** Omastorm's adapters never pass a provider's list through as the timeline, so there is nothing to thin. One cheap hardening in the same spirit: `FixtureMosaic` returns `None` for its policy (`source.rs:398`); a unit test asserting every registered polling source has `Some(policy)` with `max_frames <= 60` would keep a future adapter from shipping unbounded. Effort S, tests.

### 5. Rain-approaching notifications

**More Weather.** Settings give a strength (any/moderate/heavy as 0.1/0.51/4.01 mm/h) and a radius; the radius becomes a lead time at an assumed 50 km/h, so 25 km ≈ 30 min (`Panel.qml:979-986`). `rainAlertStart` scans the 15-minute nowcast series for the first slot within that lead time at or above the threshold, counting radar-derived slots as certain and others only at ≥ 50 % probability, and returns nothing if it already rains that hard now (`Model.js:2217-2256`; also `Panel.qml:987-990`). The series comes from Open-Meteo `minutely_15` blended with radar where available (`Model.js:2015-2017`). For Brazil More Weather has no national radar (RainViewer, `README.md:72`, `295`), so its Brazilian rain notice is model-based. `WeatherNotifications.qml:68-104` sends one `notify-send` per escalation level per two hours, persists announced keys across restarts (`WeatherNotifications.qml:12-31`), refuses to notify from cached data (`:46`, `:75`) and escapes markup from service text (`:116-120`).

**Omastorm.** Nothing found: no notifications and no nowcast. `docs/grid-adapters.md:40` lists "Forecasts or a second chrome" as a non-goal.

**Verdict: SKIP the nowcast; TAKE (idea) an observational variant later.** A lead-time forecast conflicts with the non-goal, and the 50 km/h constant is not motion. What fits Omastorm is an observation: "echo of at least N dBZ within R km of your location now", computed by the engine from the current INPE composite (the cells are already on a regular WGS84 grid, `inpe.rs:43-57`). Keep More Weather's delivery rules: notify only from live (not cached or stale) data, one notice per escalation, persisted keys, `--` before untrusted text and markup escaped. Effort M (engine check, protocol event, UI or `notify-send` from the bar). Rank it below the labelling work.

### 6. Fingerprinting the data path (`tests/radar-acquisition.test.mjs`)

**More Weather.** SHA-256 of eight whole data-path files (`Model.js`, `Providers.js`, `WeatherImageStore.qml`, …) and of 36 named functions sliced out of `Panel.qml` must equal recorded baselines (`tests/radar-acquisition.test.mjs:13-30`, `32-80`), and some files must not exist (`:27-29`). The header explains the purpose: a presentation rewrite must leave fetching, parsing, budgets, projection and cache files identical (`:7-12`).

**Omastorm.** The INPE tests already inject the HTTP getter and record every requested path (`engine/src/inpe/tests.rs:356-402`), and assert parts of the sequence: catalog first, legend counts, no frame fetch for a foreign legend (`inpe/tests.rs:472-491`, `526-527`, `584`). The UI never fetches radar data itself; the only network call in `ui/` is the IP-location lookup (`ui/PluginSession.qml:108-112`).

**Verdict: TAKE (idea), not the mechanism.** Hashing source text is brittle (any comment edit fails it, and More Weather had to re-baseline for 3.5.0, `:10-12`). Omastorm's architecture already gives the real guarantee: the UI cannot change requests because it makes none. The idea worth taking is a single golden test of one full INPE poll cycle against the mock: the exact ordered list of paths for a given catalog and clock, including what is *not* refetched on the next cycle (`docs/inpe-brazil.md`, "Timing": "nothing is refetched"). That pins request volume, which matters for an API whose terms are unrecorded. Effort S, `engine/src/inpe/tests.rs`.

### 7. `tests/no-bare-color.test.mjs` and Omastorm 04a2680

**More Weather.** Two checks over every `.qml` file: no `Color.` not preceded by a word character or dot, and every file using `Commons.Color.` has `import qs.Commons as Commons` (`tests/no-bare-color.test.mjs:1-41`). The header gives the reason: in Qt 6.12, QtQuick's own `Color` type hides the `qs.Commons` singleton, so `Color.accent` is undefined (`:1-4`).

**Omastorm.** Commit `04a2680` (#201) fixed exactly that bug in `ui/RadarBar.qml` by hand (three references, plus the import). A search today finds no bare `Color.` left in `ui/`. Nothing prevents a regression: `mise run lint` runs `qmlformat` as a syntax check and `qmllint` on `.js` only (`scripts/tooling/cli.py:134-142`); neither catches this, because the bare name resolves without error to a different type.

**Verdict: TAKE (idea).** Add the same two checks to the UI lint scope, as a few lines of Python in `scripts/tooling/cli.py` (or a `tests/` unit test). The regex and the rule are trivial and generic; reimplement rather than copy, so no attribution is needed. Effort S. High general value for the cost: the bug class shipped once already.

### 8. Feed-health semantics

**More Weather.** `RadarStyle.feedState` (`RadarStyle.js:56-65`): unreachable if the fetch failed, loading if there is no reference time, stale at 10 minutes, else live. The reference is the newest *observation* time; forecast frames count only through their embedded reference time (`Panel.qml:1514-1529`, `4147-4152`), so a nowcast frame stamped in the future cannot make a dead feed look live. The rule from the comment: "Frame age and feed health are independent; a forecast is never a fresh observation" (`RadarStyle.js:56`). Tested at the 600 000 ms edge (`tests/radar-style.test.mjs:23-26`).

**Omastorm.** Richer already. `ConnectionStatus` has ok/stale/unavailable/offline/loading/idle (`docs/protocol.md:91-100`); `feed_condition` judges age only for a reachable feed (`engine/src/main.rs:326-343`) from `evidence_age_secs`, the newest radial or frame (`main.rs:633-644`); sources declare `AgeRules` (`source.rs:35-47`), INPE 45/90 minutes because its slots are 20-30 minutes old by design (`inpe.rs:74-80`). The gap is in what INPE calls evidence: a frame's `start_ms` and `scan_time` are the *slot end* (`inpe.rs:668-675`, `877-891`), and each radar contributes its newest frame in `(slot − 20 min, slot]` (`inpe.rs:59-64`, `546-554`). So the age and the strip stamp can read up to 20 minutes fresher than the newest contributing observation, against DESIGN.md's rule that "the strip stamp is the absolute observation time" (`DESIGN.md:80-81`). A slot is also published from however many radars answered, even one (`inpe.rs:857-864`), with `status: Complete`.

**Verdict: TAKE (idea).** Apply More Weather's principle to INPE: derive the frame's evidence time from the newest contributing radar frame (the `picks` already hold each radar's time, `inpe.rs:838-844`), report the oldest-to-newest span and the contributor count in frame metadata, and let `evidence_age_secs` use that observation time instead of the slot. Consider `partial` (or a coverage note) when contributors fall well below the live layer count. Effort S, `inpe.rs` plus a protocol field. Brazil-specific value is high: with a 20-minute publish delay already, a further hidden 20 minutes is the difference between "stale" and "live" under the 45-minute rule.

### 9. Image sanitization, bounded readers and per-day budgets

**More Weather.** Needed because Qt's `Image` would buffer and decode anything: all HTTP goes through curl, which stops at `maxBytes`, with `head -c` as a second wall (`WeatherRequest.qml:5-15`, `46-56`); PNGs are rewritten chunk by chunk to IHDR/PLTE/tRNS/IDAT/IEND, with 2048 px a side, 4 Mpx, 4 MiB IDAT and 4096 chunks (`Model.js:1643-1656`, `1760-1800`; `WeatherImageStore.qml:6-15`; tested in `tests/png-sanitize.test.mjs`). Parsers ignore forged JSON `length` members and refuse oversized grids (`tests/bounded-parsers.test.mjs:1-41`). The globe has a per-UTC-day budget of Open-Meteo point-calls: from 2,000 only user-caused loads, from 3,000 none (`GlobeGrid.js:26-29`, `335-359`), shown to the user (`WeatherSourcesPage.qml:114`). CAP downloads have a per-refresh budget (`Model.js:2389`, `2515-2519`). Requests carry an identifying User-Agent (`WeatherRequest.qml:43-44`, `Providers.js:173`).

**Omastorm.** Bounded reading is already equivalent or better, in Rust: `take_body` rejects on `Content-Length` and on bytes received (`engine/src/live_index.rs:78-95`); INPE caps catalog 4 MiB, times 256 KiB, legend 64 KiB, frame 4 MiB, decode memory 16 MiB through `png::Limits`, 2048 px a side, texture 32 MiB, and 30 s per request (`inpe.rs:68`, `82-88`, `359-373`, `662-664`). The `png` crate decodes pixel data only, so More Weather's ancillary-chunk rewriting has no counterpart to fill. The UI loads only textures the engine has already decoded and re-encoded into its runtime directory (`docs/protocol.md:355-360`). What is missing: no per-day request accounting anywhere (searches for budget/quota/daily find only texture-disk and OSM geometry budgets, `main.rs:1233-1234`, `engine/src/osm/budget.rs`), and the INPE client sets no User-Agent (`inpe.rs:958-970` uses `live_index::http_client()`, a default `reqwest` client, `live_index.rs:67-76`), unlike `metar.rs:33`, `318` and `osm.rs:39`, `166`.

**Verdict: SKIP the PNG sanitizer; TAKE (idea) the budget and User-Agent.** Rough INPE volume from the code, assuming 22 live radars that each publish every 10 minutes: 720 catalog requests a day (every 120 s, `inpe.rs:67`), about 3,200 time lists (only for layers whose `latest_time` changed, `inpe.rs:806-821`) and about 3,200 frames, so roughly 7,000 requests a day per running engine, all unauthenticated against an API whose terms are not recorded. Add a counter per source per UTC day with a ceiling that stops polling and reports `offline` with a reason, log the daily total, and send the same identifying User-Agent as METAR and OSM. Effort S-M.

## Other findings worth taking

- **Fail-closed geometry.** More Weather's generic `geometryContainsPoint` returns *true* for a missing or unknown geometry (`Model.js:2794-2797`), and only the INMET caller guards against that (`Model.js:3088`). If Omastorm adds warnings, make "no usable polygon" mean "not here" in the shared function, not in each caller. S.
- **Escaping untrusted text in notifications.** `WeatherNotifications.qml:98-102`, `116-120`: `--` before arguments and markup entities for service text. Applies to any future Omastorm notification. S.

## Not worth taking

- **RainViewer.** A consumer tile service with provider colours and a smoothed scheme (`docs/radar-rebuild.md:33`); for Brazil it is what INPE's composite replaces. `docs/grid-adapters.md:35` excludes precolored consumer tiles as a default source, and Omastorm already has a better Brazilian product.
- **`Panel.qml`.** 5,112 lines, 236 KB, holding state, scheduling, radar timing and presentation together; More Weather needs `radar-acquisition.test.mjs` largely because of it. Omastorm's engine/UI split already avoids the problem; nothing to learn beyond "do not do this".
- **Hash-of-source tests** (the mechanism of item 6): brittle and redundant given the engine owns all requests.
- **Open-Meteo nowcast blending and the globe** (`Model.js:2015-2120`, `Globe*.js`): forecasts, a non-goal (`docs/grid-adapters.md:40`).
- **The PNG chunk rewriter** (`Model.js:1767`): solves a Qt `Image` problem Omastorm does not have.
- **Rain-rate to dBZ conversion** (`RadarStyle.js:10-12`, Z = 200 R^1.6): Omastorm's sources publish reflectivity; converting would lose information.

## Attribution notes

No item above recommends copying code. If any is copied, More Weather is MIT and requires the notice "Copyright (c) 2026 T-The-Coder" plus the permission text (`LICENSE`); its `LICENSE` also carries a David Heinemeier Hansson line for the Omarchy-derived widget. The pieces most likely to tempt copying, and that would need the notice: `alertHubFeeds` and `publicHttpsUrl` (`Model.js:3008-3059`), `inmetAlertReport` (`Model.js:3063-3113`), `boundedRadarFrames` (`Model.js:3391-3398`), the no-bare-color test (`tests/no-bare-color.test.mjs`). More Weather's radar palette is "after Omastorm's palette" (`RadarStyle.js:5-6`); that flows the other way and needs nothing from Omastorm.

## Sources

More Weather ([T-The-Coder/more-weather](https://github.com/T-The-Coder/more-weather) at `201f44d`, read only):

- `LICENSE`, `README.md`, `PROVIDERS.md`, `docs/radar-rebuild.md`, `i18n/en.js`
- `Model.js` (lines 1643-1800, 2015-2256, 2380-2530, 2793-2830, 2890-3113, 3383-3440)
- `Providers.js` (173, 315-336, 395-460), `RadarStyle.js`, `RadarMotion.mjs`, `GlobeGrid.js`
- `Panel.qml` (975-1010, 1246, 1500-1560, 1925-1940, 4147-4152), `WeatherAlertLookup.qml`, `WeatherNotifications.qml`, `WeatherRequest.qml`, `WeatherImageStore.qml`, `WeatherRadarLegend.qml`, `WeatherSourcesPage.qml`, `WeatherGlobeData.qml`
- `tests/radar-acquisition.test.mjs`, `tests/no-bare-color.test.mjs`, `tests/bounded-parsers.test.mjs`, `tests/png-sanitize.test.mjs`, `tests/radar-style.test.mjs`, `tests/model.test.mjs`

Omastorm (this repository):

- `engine/src/inpe.rs`, `engine/src/inpe/tests.rs`, `engine/src/main.rs`, `engine/src/source.rs`, `engine/src/catalog.rs`, `engine/src/live_index.rs`, `engine/src/opera.rs`, `engine/src/mrms.rs`, `engine/src/metar.rs`, `engine/src/osm.rs`, `engine/src/osm/budget.rs`, `engine/src/tiles.rs`
- `ui/RadarWindow.qml`, `ui/RadarBar.qml`, `ui/Popover.qml`, `ui/PluginSession.qml`
- `README.md`, `DESIGN.md`, `docs/protocol.md`, `docs/grid-adapters.md`, `docs/inpe-brazil.md`, `mise.toml`, `scripts/tooling/cli.py`
- `git show 04a2680`

Network (one request each, curl defaults, no extra headers):

- `GET https://apiprevmet3.inmet.gov.br/avisos/ativos`: connection reset, no response (see "INMET live probe").
- `GET https://example.com/`: 200, only to rule out a local network fault.
- DNS lookup of `apiprevmet3.inmet.gov.br`.

Referenced but not fetched: `https://alert-hub-sources.s3.amazonaws.com/json`, `https://alertingauthority.wmo.int/`, `https://alert-hub.org/`, `https://alertas2.inmet.gov.br/`.
