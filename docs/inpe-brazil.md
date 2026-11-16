# INPE Brazil composite

`engine/src/inpe.rs` builds the `inpe-brazil` source: a **composite**, a
mosaic Omastorm assembles itself, of Brazil's radars from the per-radar
products that INPE/CPTEC publishes for its Nowcasting map. Its tests,
`engine/src/inpe/tests.rs`, synthesize every input and run offline.

Related: [grid-adapters.md](grid-adapters.md) (the GridFamily contract this
source implements) and [protocol.md](protocol.md).

## Data

INPE's [Nowcasting](https://nowcasting.cptec.inpe.br/) API serves, without a
key, a catalog of map layers and, for each radar layer, its frame times, its
legend and one PNG per frame:

```text
https://nowcasting.cptec.inpe.br/api/v1/paths/catalog
https://nowcasting.cptec.inpe.br/api/v1/paths/radar/layers/<layer>/times
https://nowcasting.cptec.inpe.br/api/v1/paths/radar/layers/<layer>/legend
https://nowcasting.cptec.inpe.br/api/v1/paths/frames/radar/<layer>/<time>/full
```

The adapter uses one product: CAPPI 3 km reflectivity, the layers whose name
ends in `~cappi~cappi3km` under the catalog's `radar` source. Layer names
carry the operating network as a prefix (`apac~chagrande~cappi~cappi3km`,
`sdcsc~chapeco~cappi~cappi3km`). On 2026-11-15 the catalog listed 32 such
layers, 22 of them live, from DECEA, CEMADEN, SIPAM and state networks. INPE
publishes no national mosaic and no measured-value raster, only these
precolored PNGs, which its own Leaflet client stretches over each layer's
published latitude/longitude box.

## Composite

**Grid.** WGS84 longitude/latitude, 0.011° cells (about 1.2 km, the radars'
1 km product at these latitudes), 3840 × 3550 cells over the
`INPE_BRAZIL` box in `engine/src/envelope.rs`: north 5.2°, south −33.85°,
west −75.2°, east −32.96°. That box encloses every CAPPI 3 km layer box listed
on 2026-11-15. The grid stays under 4096 px on both axes, so it needs no GPU
readback check (`docs/grid-adapters.md`).

**Placement.** Leaflet places an image linearly in Web Mercator between its
box's north and south edges, and linearly in longitude between west and east.
The adapter samples each radar the same way: a composite row maps to a source
row through the Mercator northing, a column to a source column linearly. A
cell takes a radar's value only inside that radar's range circle, derived
from its box. Verified on 2026-11-15 by overlaying a live composite on Natural
Earth coastlines and state lines: every coverage circle sat on its site.

**Values.** Each layer's legend lists 74 entries at 1 dBZ resolution (0–4 dBZ
share white). A layer is used only when its live legend matches the accepted
profile in `LEGEND` exactly; a changed legend skips that radar rather than
reinterpreting it. Each opaque pixel is projected onto the legend ramp: a
legend colour gives its value, a resampled blend of two adjacent colours
interpolates between them, and a colour further than 16 RGB units from the
whole ramp matches nothing. A frame with more than 2 % unmatched opaque pixels
is rejected as a foreign palette. Transparent pixels are no echo. The result is
**legend-derived reflectivity**, and the product says so: "CAPPI 3 km
Reflectivity (composite from INPE legend)", in dBZ. Values are stored as
OPERA-style half-dBZ codes (`floor(dBZ × 2 + 66)`), so the shared palette,
bounds and measured-value readout apply unchanged.

**Overlap.** Every cell keeps the strongest code of the radars that reach it.
Cells inside some radar's range with no echo are *undetect*; cells no radar
reaches are *missing*. The output is the shared classified RGBA texture: class
in R, missing/undetect flags in G, half-dBZ code in B.

## Timing

Each composite carries a **nominal time** on a ten-minute UTC boundary, as
OPERA and MRMS carry their providers' nominal times. It stands for the
20 minutes before it: each radar contributes its newest frame in
`(nominal − 20 min, nominal]`, so the observations in a composite are up to
20 minutes older than its stamp. INPE frames reach the API 10–40
minutes after observation, so a slot is built only 20 minutes after it ends,
and is never revised once published. A layer whose catalog `latest_time` is
more than three hours old is treated as offline.

The adapter polls every 120 seconds. A cycle refreshes the catalog, fetches
time lists only for layers whose `latest_time` changed, then builds every
unpublished slot newest first: the newest as the live frame, older ones as
history, up to 12 slots (two hours). Rasters are cached for reuse by the next
older slot; nothing is refetched. A test pins this request order.

Requests identify themselves with Omastorm's User-Agent, as the METAR and
map clients do. A steady day is about 7,000 requests: 720 catalog reads,
roughly 3,200 time lists (only for layers whose newest frame changed) and
roughly 3,200 frames of 10–40 KB.

Feed age uses the source's own rules (`inpe::AGE`): stale at 45 minutes,
unavailable at 90. NEXRAD's 10/30-minute rules would always read stale here,
because the newest slot is 20–30 minutes old by design.

## Bounds and validation

Every request has a 30-second timeout and a byte cap: catalog 4 MiB, times
256 KiB, legend 64 KiB, frame 4 MiB. Frame PNGs are limited to 2048 px a side
and 16 MiB of decode memory; the composite texture to 32 MiB. Layer names
enter URL paths only after a character allowlist. One composite builds at a
time, on a blocking thread, holding a permit that a cancelled session keeps
until its work actually exits. Network work starts in `poll`, never at
construction. An unreachable catalog reports the source offline and keeps
retained frames; a catalog with no recent CAPPI 3 km frames reports silence.

## Selection

`inpe-brazil` is a covering mosaic with selection priority 20, so follow
picks it anywhere inside its box. It must stay above any continental fallback
compiled in later. Nothing outside Brazil's box changes.

## Precoloured source

[grid-adapters.md](grid-adapters.md#non-goals) excludes precoloured imagery,
with one exception that this source meets: a national product with no
measured-value alternative, whose published legend matches an accepted
profile exactly for every layer, and whose frames are rejected when unknown
colours appear. Manual-only selection was rejected: nothing else covers
Brazil, so follow would show an empty map there.

## Open

- **Access and license terms.** The grid-adapter rules require a provider's
  terms before an adapter is compiled in, and an attribution entry in README.
  INPE's terms of use for the Nowcasting API are not yet recorded: none were
  found on the Nowcasting site or API on 2026-11-16, and the gov.br portal's
  CC BY-ND 3.0 footer covers site content, not necessarily this data. Ask
  nowcasting.cptec@inpe.br. Nothing is vendored: frames are fetched live and
  cached at runtime only.

## Verifying

```sh
bash scripts/cargo.sh test inpe::
python3 scripts/tooling/probe.py inpe-brazil --seconds 60   # live
```
