//! INPE/CPTEC Nowcasting radar composite for Brazil (`docs/inpe-brazil.md`).
//!
//! INPE publishes each radar's CAPPI 3 km reflectivity as a precolored PNG
//! that its own Leaflet client stretches over a latitude/longitude box in Web
//! Mercator. There is no national mosaic and no measured-value product, so
//! this adapter inverts the published 1 dBZ legend, resamples every live radar
//! onto one WGS84 latitude/longitude grid, and keeps the strongest return per
//! cell. All network work starts in `poll`, never in construction.

use crate::{
    envelope::INPE_BRAZIL,
    live_index,
    opera::class_of,
    protocol::{Coverage, Crs, Family, FrameStatus, Kind, MeasuredCode, MosaicFrame, ProductClass},
    source::{AgeRules, GridEvent, GridSender, HistoryPolicy, MosaicMeta, SourceMetadataBorrowed},
    sweep,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    io::{Cursor, Write},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::Semaphore,
    task::{JoinHandle, spawn_blocking},
    time::{sleep, timeout},
};

pub const ID: &str = "inpe-brazil";
pub const NAME: &str = "INPE/CPTEC — Brazil radar composite";
pub const ATTRIBUTION: &str = "INPE/CPTEC Nowcasting · DECEA, CEMADEN, SIPAM and state radars";
/// National composite; nothing else covers Brazil today, but it must still
/// outrank a continental fallback should one be compiled in.
const SELECTION_PRIORITY: i32 = 20;
const HOST: &str = "https://nowcasting.cptec.inpe.br/api/v1/paths/";
/// The one product every INPE radar layer publishes with the same legend.
const LAYER_SUFFIX: &str = "~cappi~cappi3km";

/// Composite grid: WGS84 longitude/latitude, 0.011° cells (about 1.2 km, the
/// radars' 1 km product at these latitudes), under 4096 px on both axes.
pub const STEP_DEG: f64 = 0.011;
pub const WIDTH: u32 = 3840;
pub const HEIGHT: u32 = 3550;
// Larger mosaics need TextureCheck.qml's GPU readback (`docs/grid-adapters.md`).
const _: () = assert!(WIDTH <= 4096 && HEIGHT <= 4096);
pub const GEOTRANSFORM: [f64; 6] = [
    INPE_BRAZIL.west,
    STEP_DEG,
    0.0,
    INPE_BRAZIL.north,
    0.0,
    -STEP_DEG,
];

/// Composites are stamped on ten-minute slots. Each radar contributes its
/// newest frame in `(slot - WINDOW, slot]`; a slot is built once the slowest
/// usual radar has had time to deliver, then never revised.
const SLOT_MS: i64 = 10 * 60 * 1000;
const WINDOW_MS: i64 = 20 * 60 * 1000;
const PUBLISH_DELAY_MS: i64 = 20 * 60 * 1000;
/// A layer whose newest frame is older than this is treated as offline.
const LIVE_WITHIN_MS: i64 = 3 * 60 * 60 * 1000;
const POLL_INTERVAL: Duration = Duration::from_secs(120);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

pub const HISTORY: HistoryPolicy = HistoryPolicy {
    max_frames: 12,
    window_ms: Some(12 * SLOT_MS),
};
/// INPE frames reach the API 10-40 minutes after observation and composites
/// wait for the slow ones, so the NEXRAD 10/30 minute rules would always read
/// stale. These still flag a feed that stopped updating.
pub const AGE: AgeRules = AgeRules {
    stale_after_s: 45 * 60,
    unavailable_after_s: 90 * 60,
};

const CATALOG_MAX: usize = 4 << 20;
const TIMES_MAX: usize = 256 << 10;
const LEGEND_MAX: usize = 64 << 10;
const FRAME_MAX: usize = 4 << 20;
const PNG_MEMORY_MAX: usize = 16 << 20;
const FRAME_DIM_MAX: u32 = 2048;
const TEXTURE_MAX: usize = 32 << 20;
/// RGB distance from the legend ramp. Verified samples sit within 12; the
/// resampled blends between adjacent legend colours are on the ramp itself.
const COLOUR_TOLERANCE: f32 = 16.0;
/// Opaque pixels matching no legend colour, per mille, before a frame is
/// rejected as an unexpected palette rather than silently reinterpreted.
const UNKNOWN_MAX_PER_MILLE: usize = 20;
const KM_PER_DEG: f64 = 111.195;

/// Half-dBZ measured codes, as OPERA: B = floor(value * 2 + 66), 2..255.
const CODE: MeasuredCode = MeasuredCode {
    scale: 2.0,
    offset: 66.0,
};
/// Composite cell codes: no contributing radar reaches the cell, a radar
/// covers it and saw nothing, or a measured half-dBZ code (>= 2). Ordered so
/// the composite is a plain maximum.
const MISSING: u8 = 0;
const UNDETECT: u8 = 1;

/// The accepted `cappi3km` legend, dBZ and colour, identical across every
/// INPE radar layer when verified on 2026-11-15. A layer whose live legend
/// differs is skipped rather than reinterpreted.
const LEGEND: [(f32, u32); 74] = [
    (0.0, 0xffffff),
    (1.0, 0xffffff),
    (2.0, 0xffffff),
    (3.0, 0xffffff),
    (4.0, 0xffffff),
    (5.0, 0xc6cccc),
    (6.0, 0xbbc1c1),
    (7.0, 0xa3abab),
    (8.0, 0x8f9a9a),
    (9.0, 0x7c8989),
    (10.0, 0x00f3f8),
    (11.0, 0x00d8ec),
    (12.0, 0x00bde0),
    (13.0, 0x00a2d4),
    (14.0, 0x0087c8),
    (15.0, 0x006cbc),
    (16.0, 0x0051b0),
    (17.0, 0x0036a4),
    (18.0, 0x001b98),
    (19.0, 0x00008c),
    (20.0, 0x23ff00),
    (21.0, 0x00f200),
    (22.0, 0x00e600),
    (23.0, 0x00d900),
    (24.0, 0x00ca00),
    (25.0, 0x00b400),
    (26.0, 0x00a800),
    (27.0, 0x009b00),
    (28.0, 0x008f00),
    (29.0, 0x008200),
    (30.0, 0x007600),
    (31.0, 0x006900),
    (32.0, 0x005d00),
    (33.0, 0x005100),
    (34.0, 0x004800),
    (35.0, 0xffff00),
    (36.0, 0xffed00),
    (37.0, 0xffe400),
    (38.0, 0xffdb00),
    (39.0, 0xffd200),
    (40.0, 0xffc900),
    (41.0, 0xffc000),
    (42.0, 0xffb700),
    (43.0, 0xffae00),
    (44.0, 0xffa500),
    (45.0, 0xff9c00),
    (46.0, 0xff9300),
    (47.0, 0xff8a00),
    (48.0, 0xff8100),
    (49.0, 0xff7800),
    (50.0, 0xf30000),
    (51.0, 0xe70000),
    (52.0, 0xdb0000),
    (53.0, 0xcf0000),
    (54.0, 0xc30000),
    (55.0, 0xb70000),
    (56.0, 0xab0000),
    (57.0, 0x9f0000),
    (58.0, 0x930000),
    (59.0, 0x870000),
    (60.0, 0x7b0000),
    (61.0, 0x6f0000),
    (62.0, 0x630000),
    (63.0, 0x570000),
    (64.0, 0xe11ea7),
    (65.0, 0xdd22b8),
    (66.0, 0xdd22ca),
    (67.0, 0xdd22dd),
    (68.0, 0xca22dd),
    (69.0, 0xb822dd),
    (70.0, 0xa522dd),
    (71.0, 0x9322dd),
    (72.0, 0x8022dd),
    (73.0, 0x8022dd),
];

pub struct Inpe {
    pub id: &'static str,
    coverage: Coverage,
    palette: Arc<Vec<String>>,
    bounds: Arc<Vec<f64>>,
    // On the registry adapter, not the poll task: a cancelled session's
    // blocking composite keeps its permit until it actually exits.
    work: Arc<Semaphore>,
}

impl Inpe {
    pub fn new() -> Self {
        let template: crate::protocol::Frame =
            serde_json::from_str(include_str!("../data/product.json")).unwrap();
        Self {
            id: ID,
            coverage: Coverage::Box {
                north: INPE_BRAZIL.north,
                south: INPE_BRAZIL.south,
                west: INPE_BRAZIL.west,
                east: INPE_BRAZIL.east,
            },
            palette: Arc::new(template.palette),
            bounds: Arc::new(template.bounds.into_iter().map(f64::from).collect()),
            work: Arc::new(Semaphore::new(1)),
        }
    }

    pub fn metadata(&self) -> SourceMetadataBorrowed<'_> {
        SourceMetadataBorrowed {
            id: ID,
            family: Family::Grid,
            kind: Kind::Mosaic,
            default_product_class: ProductClass::Reflectivity,
            name: NAME,
            attribution: ATTRIBUTION,
            mosaic: Some(MosaicMeta {
                coverage: &self.coverage,
                selection_priority: SELECTION_PRIORITY,
                covering: true,
            }),
        }
    }

    pub fn loading_placeholder(&self) -> Option<(MosaicFrame, Vec<u8>)> {
        let mut frame = frame(0, &self.palette, &self.bounds);
        frame.id = format!("{ID}-loading");
        frame.scan_time = String::new();
        frame.status = FrameStatus::Partial;
        frame.width = 1;
        frame.height = 1;
        frame.geotransform = [
            INPE_BRAZIL.west,
            INPE_BRAZIL.east - INPE_BRAZIL.west,
            0.0,
            INPE_BRAZIL.north,
            0.0,
            INPE_BRAZIL.south - INPE_BRAZIL.north,
        ];
        frame.measured_code = None;
        Some((frame, sweep::png(1, 1, &[0, 1, 0, 255]).ok()?))
    }

    pub fn poll(&self, events: GridSender, retained: Vec<i64>) -> JoinHandle<()> {
        tokio::spawn(poll_loop(
            events,
            retained,
            get_http,
            Arc::clone(&self.work),
            Arc::clone(&self.palette),
            Arc::clone(&self.bounds),
            POLL_INTERVAL,
            now_ms,
        ))
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn inpe_log(message: impl std::fmt::Display) {
    eprintln!("{} INPE {message}", Utc::now().to_rfc3339());
}

// ---- Legend inversion -------------------------------------------------------

fn rgb(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 0xff) as f32,
        ((hex >> 8) & 0xff) as f32,
        (hex & 0xff) as f32,
    ]
}

/// dBZ for a legend colour, or for a resampled blend of two adjacent legend
/// colours (projected onto the segment between them); `None` when the colour
/// is further than [`COLOUR_TOLERANCE`] from the whole ramp. Repeated legend
/// colours (0-4 dBZ are all white) resolve to the lowest value.
pub(crate) fn invert(pixel: [u8; 3]) -> Option<f32> {
    let c = pixel.map(f32::from);
    let mut best = (f32::INFINITY, 0.0f32);
    for (i, &(value, hex)) in LEGEND.iter().enumerate() {
        let a = rgb(hex);
        let (next_value, b) = LEGEND.get(i + 1).map_or((value, a), |&(v, h)| (v, rgb(h)));
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let t = if len2 > 0.0 {
            ((ac[0] * ab[0] + ac[1] * ab[1] + ac[2] * ab[2]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = [ac[0] - t * ab[0], ac[1] - t * ab[1], ac[2] - t * ab[2]];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if distance < best.0 {
            best = (distance, value + t * (next_value - value));
        }
    }
    (best.0 <= COLOUR_TOLERANCE).then_some(best.1)
}

/// Floor rather than round, so a value just below a half-dBZ edge stays below.
pub(crate) fn measured_code(value: f32) -> u8 {
    (f64::from(value) * f64::from(CODE.scale) + f64::from(CODE.offset))
        .floor()
        .clamp(2.0, 255.0) as u8
}

#[derive(Deserialize)]
struct Legend {
    entries: Vec<LegendEntry>,
}

#[derive(Deserialize)]
struct LegendEntry {
    value: f32,
    color: String,
}

/// Whether a layer's published legend is exactly the accepted profile.
pub(crate) fn legend_matches(bytes: &[u8]) -> bool {
    let Ok(legend) = serde_json::from_slice::<Legend>(bytes) else {
        return false;
    };
    legend.entries.len() == LEGEND.len()
        && legend
            .entries
            .iter()
            .zip(LEGEND)
            .all(|(entry, (value, hex))| {
                entry.value == value
                    && entry.color.len() == 7
                    && entry
                        .color
                        .strip_prefix('#')
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        == Some(hex)
            })
}

// ---- One radar frame --------------------------------------------------------

/// One radar's frame as measured codes: 0 where INPE drew nothing.
#[derive(Debug, PartialEq)]
pub(crate) struct RadarRaster {
    pub width: usize,
    pub height: usize,
    pub codes: Vec<u8>,
}

pub(crate) fn decode_radar(bytes: &[u8]) -> Result<RadarRaster, String> {
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: PNG_MEMORY_MAX,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("INPE PNG header: {e}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > FRAME_DIM_MAX || height > FRAME_DIM_MAX {
        return Err(format!("INPE PNG is {width}x{height}"));
    }
    let size = reader
        .output_buffer_size()
        .ok_or("INPE PNG output too large")?;
    let mut buffer = vec![0; size];
    let out = reader
        .next_frame(&mut buffer)
        .map_err(|e| format!("INPE PNG pixels: {e}"))?;
    let channels = match (out.color_type, out.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (color, depth) => return Err(format!("unsupported INPE PNG {color:?} {depth:?}")),
    };
    let (w, h) = (width as usize, height as usize);
    let mut codes = vec![0u8; w * h];
    let mut seen: HashMap<[u8; 3], Option<u8>> = HashMap::new();
    let (mut opaque, mut unknown) = (0usize, 0usize);
    for (row, line) in buffer.chunks(out.line_size).take(h).enumerate() {
        for (col, px) in line.chunks_exact(channels).take(w).enumerate() {
            if channels == 4 && px[3] < 128 {
                continue;
            }
            opaque += 1;
            let colour = [px[0], px[1], px[2]];
            match *seen
                .entry(colour)
                .or_insert_with(|| invert(colour).map(measured_code))
            {
                Some(code) => codes[row * w + col] = code,
                None => unknown += 1,
            }
        }
    }
    if unknown * 1000 > opaque * UNKNOWN_MAX_PER_MILLE {
        return Err(format!(
            "{unknown} of {opaque} INPE pixels match no legend colour"
        ));
    }
    Ok(RadarRaster {
        width: w,
        height: h,
        codes,
    })
}

// ---- Catalog ----------------------------------------------------------------

/// One live radar layer and its published placement box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Radar {
    pub layer: String,
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
    pub latest_ms: i64,
}

impl Radar {
    fn centre(&self) -> (f64, f64) {
        (
            (self.south + self.north) / 2.0,
            (self.west + self.east) / 2.0,
        )
    }
    /// The box is the latitude/longitude envelope of a square Cartesian grid
    /// centred on the dish, so its half-height is the product's range.
    fn range_km(&self) -> f64 {
        (self.north - self.south) / 2.0 * KM_PER_DEG
    }
}

#[derive(Deserialize)]
struct Catalog {
    sources: Vec<CatalogSource>,
}

#[derive(Deserialize)]
struct CatalogSource {
    slug: String,
    #[serde(default)]
    products: Vec<CatalogProduct>,
}

#[derive(Deserialize)]
struct CatalogProduct {
    layer: String,
    #[serde(default)]
    bounds: Option<[[f64; 2]; 2]>,
    #[serde(default)]
    latest_time: Option<String>,
}

fn parse_time(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// Layer names go into URL paths; accept only what INPE uses today.
fn valid_layer(layer: &str) -> bool {
    !layer.is_empty()
        && layer.len() <= 96
        && layer
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"~_-".contains(&b))
}

/// Live CAPPI 3 km radars whose box lies inside the composite grid.
pub(crate) fn parse_catalog(bytes: &[u8], now_ms: i64) -> Result<Vec<Radar>, String> {
    let catalog: Catalog =
        serde_json::from_slice(bytes).map_err(|e| format!("INPE catalog: {e}"))?;
    let source = catalog
        .sources
        .into_iter()
        .find(|s| s.slug == "radar")
        .ok_or("INPE catalog has no radar source")?;
    let mut radars: Vec<Radar> = source
        .products
        .into_iter()
        .filter_map(|p| radar(p, now_ms))
        .collect();
    radars.sort_by(|a, b| a.layer.cmp(&b.layer));
    radars.dedup_by(|a, b| a.layer == b.layer);
    Ok(radars)
}

fn radar(product: CatalogProduct, now_ms: i64) -> Option<Radar> {
    if !product.layer.ends_with(LAYER_SUFFIX) || !valid_layer(&product.layer) {
        return None;
    }
    let [[south, west], [north, east]] = product.bounds?;
    let latest_ms = parse_time(product.latest_time.as_deref()?)?;
    let inside = south >= INPE_BRAZIL.south
        && north <= INPE_BRAZIL.north
        && west >= INPE_BRAZIL.west
        && east <= INPE_BRAZIL.east;
    let sized = (0.5..=10.0).contains(&(north - south)) && (0.5..=12.0).contains(&(east - west));
    (latest_ms >= now_ms - LIVE_WITHIN_MS && inside && sized).then_some(Radar {
        layer: product.layer,
        south,
        west,
        north,
        east,
        latest_ms,
    })
}

#[derive(Deserialize)]
struct Times {
    times: Vec<String>,
}

/// Whole-minute frame times (the frame URL addresses minutes), ascending.
pub(crate) fn parse_times(bytes: &[u8]) -> Result<Vec<i64>, String> {
    let times: Times = serde_json::from_slice(bytes).map_err(|e| format!("INPE times: {e}"))?;
    let mut times: Vec<i64> = times
        .times
        .iter()
        .filter_map(|t| parse_time(t))
        .filter(|ms| ms.rem_euclid(60_000) == 0)
        .collect();
    times.sort_unstable();
    times.dedup();
    Ok(times)
}

// ---- Slots ------------------------------------------------------------------

pub(crate) fn newest_slot(now_ms: i64) -> i64 {
    (now_ms - PUBLISH_DELAY_MS).div_euclid(SLOT_MS) * SLOT_MS
}

/// The radar's newest frame in `(slot - WINDOW, slot]`.
pub(crate) fn pick(times: &[i64], slot: i64) -> Option<i64> {
    times
        .iter()
        .rev()
        .copied()
        .find(|&t| t <= slot)
        .filter(|&t| t > slot - WINDOW_MS)
}

// ---- Composite --------------------------------------------------------------

/// Web Mercator northing in radians: INPE's Leaflet client places each PNG
/// linearly in this space between the box's north and south edges.
fn mercator(lat_deg: f64) -> f64 {
    (std::f64::consts::FRAC_PI_4 + lat_deg.to_radians() / 2.0)
        .tan()
        .ln()
}

pub(crate) fn composite(layers: &[(&Radar, &RadarRaster)]) -> Vec<u8> {
    let mut cells = vec![MISSING; WIDTH as usize * HEIGHT as usize];
    for (radar, raster) in layers {
        splat(&mut cells, radar, raster);
    }
    cells
}

/// Nearest-neighbour resample of one radar into the composite: cells within
/// its range are at least `UNDETECT`, and the strongest code wins.
fn splat(cells: &mut [u8], radar: &Radar, raster: &RadarRaster) {
    let [x0, dx, _, y0, _, dy] = GEOTRANSFORM;
    let (width, height) = (WIDTH as usize, HEIGHT as usize);
    let (top, bottom) = (mercator(radar.north), mercator(radar.south));
    let (lat_c, lon_c) = radar.centre();
    let range2 = radar.range_km().powi(2);
    let col0 = ((radar.west - x0) / dx).floor().max(0.0) as usize;
    let col1 = (((radar.east - x0) / dx).ceil().max(0.0) as usize).min(width);
    let row0 = ((radar.north - y0) / dy).floor().max(0.0) as usize;
    let row1 = (((radar.south - y0) / dy).ceil().max(0.0) as usize).min(height);
    let (sw, sh) = (raster.width as f64, raster.height as f64);
    for row in row0..row1 {
        let lat = y0 + (row as f64 + 0.5) * dy;
        let y = (top - mercator(lat)) / (top - bottom) * sh;
        if !(0.0..sh).contains(&y) {
            continue;
        }
        let source = &raster.codes[y as usize * raster.width..][..raster.width];
        let north_km = (lat - lat_c) * KM_PER_DEG;
        let east_scale = KM_PER_DEG * lat.to_radians().cos();
        let target = &mut cells[row * width..][..width];
        for (col, cell) in target.iter_mut().enumerate().take(col1).skip(col0) {
            let lon = x0 + (col as f64 + 0.5) * dx;
            let east_km = (lon - lon_c) * east_scale;
            if east_km * east_km + north_km * north_km > range2 {
                continue;
            }
            let x = (lon - radar.west) / (radar.east - radar.west) * sw;
            if !(0.0..sw).contains(&x) {
                continue;
            }
            *cell = (*cell).max(source[x as usize].max(UNDETECT));
        }
    }
}

fn texels(bounds: &[f64], classes: usize) -> [[u8; 4]; 256] {
    let classes = classes.max(1);
    let mut lut = [[0, 1, 0, 255]; 256];
    lut[usize::from(UNDETECT)] = [0, 2, 0, 255];
    for code in 2..=255u8 {
        let value = (f64::from(code) - f64::from(CODE.offset)) / f64::from(CODE.scale);
        lut[usize::from(code)] = [class_of(value, bounds, classes) + 1, 0, code, 255];
    }
    lut
}

/// Classified RGBA texture: R class + 1, G bit 0 missing / bit 1 undetect,
/// B the half-dBZ code. Streams row by row; no full RGBA plane.
pub(crate) fn encode(
    cells: &[u8],
    width: u32,
    height: u32,
    bounds: &[f64],
    classes: usize,
) -> Result<Vec<u8>, String> {
    if cells.len() != width as usize * height as usize {
        return Err("INPE composite size mismatch".into());
    }
    let lut = texels(bounds, classes);
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("INPE texture header: {e}"))?;
    let mut stream = writer
        .stream_writer_with_size(64 << 10)
        .map_err(|e| format!("INPE texture stream: {e}"))?;
    let mut line = vec![0u8; width as usize * 4];
    for row in cells.chunks_exact(width as usize) {
        for (texel, &code) in line.as_chunks_mut::<4>().0.iter_mut().zip(row) {
            texel.copy_from_slice(&lut[usize::from(code)]);
        }
        stream
            .write_all(&line)
            .map_err(|e| format!("INPE texture pixels: {e}"))?;
    }
    stream
        .finish()
        .map_err(|e| format!("INPE texture stream end: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("INPE texture trailer: {e}"))?;
    if output.len() > TEXTURE_MAX {
        return Err("INPE texture over the size limit".into());
    }
    Ok(output)
}

pub(crate) fn frame(slot_ms: i64, palette: &[String], bounds: &[f64]) -> MosaicFrame {
    let t = DateTime::<Utc>::from_timestamp_millis(slot_ms).unwrap_or_default();
    MosaicFrame {
        id: format!("{ID}-{}", t.format("%Y%m%dT%H%M%SZ")),
        product: "REF".into(),
        product_name: "CAPPI 3 km Reflectivity (composite)".into(),
        units: "dBZ".into(),
        scan_time: t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        sweep_end: None,
        status: FrameStatus::Complete,
        texture: String::new(),
        width: WIDTH,
        height: HEIGHT,
        crs: Crs::wgs84_geographic(),
        geotransform: GEOTRANSFORM,
        palette: palette.to_vec(),
        bounds: bounds.to_vec(),
        measured_code: Some(CODE),
    }
}

// ---- Polling ----------------------------------------------------------------

/// What one poll session remembers between cycles.
#[derive(Default)]
struct Feed {
    /// Layer -> (catalog `latest_time` when listed, frame times).
    times: HashMap<String, (i64, Vec<i64>)>,
    /// Layer -> whether its legend is the accepted profile.
    legends: HashMap<String, bool>,
    rasters: HashMap<(String, i64), Arc<RadarRaster>>,
    /// Slots already published (or retained from an earlier session).
    done: BTreeSet<i64>,
    newest_sent: Option<i64>,
}

enum Outcome {
    // The count is asserted by tests; the loop only needs to know it ran.
    Published(#[cfg_attr(not(test), expect(dead_code))] usize),
    Closed,
}

async fn fetch<G, GF>(get: &G, path: String, max: usize) -> Result<Vec<u8>, String>
where
    G: Fn(String, usize) -> GF,
    GF: Future<Output = Result<Vec<u8>, String>>,
{
    match timeout(HTTP_TIMEOUT, get(path.clone(), max)).await {
        Ok(result) => result,
        Err(_) => Err(format!("INPE {path}: timed out")),
    }
}

async fn raster<G, GF>(
    feed: &mut Feed,
    get: &G,
    radar: &Radar,
    t: i64,
) -> Result<Arc<RadarRaster>, String>
where
    G: Fn(String, usize) -> GF,
    GF: Future<Output = Result<Vec<u8>, String>>,
{
    let key = (radar.layer.clone(), t);
    if let Some(raster) = feed.rasters.get(&key) {
        return Ok(Arc::clone(raster));
    }
    match feed.legends.get(&radar.layer) {
        Some(true) => {}
        Some(false) => return Err("legend differs from the accepted profile".into()),
        None => {
            let path = format!("radar/layers/{}/legend", radar.layer);
            let bytes = fetch(get, path, LEGEND_MAX).await?;
            let accepted = legend_matches(&bytes);
            feed.legends.insert(radar.layer.clone(), accepted);
            if !accepted {
                return Err("legend differs from the accepted profile".into());
            }
        }
    }
    let stamp = DateTime::<Utc>::from_timestamp_millis(t)
        .ok_or("frame time out of range")?
        .format("%Y%m%d%H%M");
    let path = format!("frames/radar/{}/{stamp}/full", radar.layer);
    let bytes = fetch(get, path, FRAME_MAX).await?;
    let raster = spawn_blocking(move || decode_radar(&bytes))
        .await
        .map_err(|e| format!("decoding: {e}"))??;
    let raster = Arc::new(raster);
    feed.rasters.insert(key, Arc::clone(&raster));
    Ok(raster)
}

async fn build(
    layers: Vec<(Radar, Arc<RadarRaster>)>,
    slot: i64,
    work: &Arc<Semaphore>,
    palette: &Arc<Vec<String>>,
    bounds: &Arc<Vec<f64>>,
) -> Result<(MosaicFrame, Vec<u8>), String> {
    let permit = Arc::clone(work)
        .acquire_owned()
        .await
        .map_err(|e| format!("INPE composite: {e}"))?;
    let (palette, bounds) = (Arc::clone(palette), Arc::clone(bounds));
    spawn_blocking(move || {
        let _permit = permit;
        let refs: Vec<(&Radar, &RadarRaster)> =
            layers.iter().map(|(r, x)| (r, x.as_ref())).collect();
        let cells = composite(&refs);
        drop(refs);
        drop(layers);
        let png = encode(&cells, WIDTH, HEIGHT, &bounds, palette.len())?;
        Ok((frame(slot, &palette, &bounds), png))
    })
    .await
    .map_err(|e| format!("INPE composite: {e}"))?
}

/// One discovery pass: refresh the catalog and changed time lists, then build
/// every unpublished slot newest first. The newest unseen slot goes out as a
/// live frame, older ones as history.
async fn cycle<G, GF>(
    feed: &mut Feed,
    now_ms: i64,
    get: &G,
    events: &GridSender,
    work: &Arc<Semaphore>,
    palette: &Arc<Vec<String>>,
    bounds: &Arc<Vec<f64>>,
) -> Result<Outcome, String>
where
    G: Fn(String, usize) -> GF,
    GF: Future<Output = Result<Vec<u8>, String>>,
{
    let catalog = fetch(get, "catalog".into(), CATALOG_MAX).await?;
    let radars = parse_catalog(&catalog, now_ms)?;
    drop(catalog);
    for radar in &radars {
        if feed
            .times
            .get(&radar.layer)
            .is_some_and(|(latest, _)| *latest == radar.latest_ms)
        {
            continue;
        }
        let path = format!("radar/layers/{}/times", radar.layer);
        match fetch(get, path, TIMES_MAX)
            .await
            .and_then(|b| parse_times(&b))
        {
            Ok(times) => {
                feed.times
                    .insert(radar.layer.clone(), (radar.latest_ms, times));
            }
            Err(e) => inpe_log(format_args!("{}: {e}", radar.layer)),
        }
    }
    feed.times
        .retain(|layer, _| radars.iter().any(|r| &r.layer == layer));

    let newest = newest_slot(now_ms);
    let oldest = newest - (HISTORY.max_frames as i64 - 1) * SLOT_MS;
    feed.done.retain(|&s| s >= oldest);
    let mut published = 0;
    for k in 0..HISTORY.max_frames as i64 {
        let slot = newest - k * SLOT_MS;
        if feed.done.contains(&slot) {
            continue;
        }
        let picks: Vec<(Radar, i64)> = radars
            .iter()
            .filter_map(|r| {
                let (_, times) = feed.times.get(&r.layer)?;
                Some((r.clone(), pick(times, slot)?))
            })
            .collect();
        let mut layers = Vec::with_capacity(picks.len());
        for (radar, t) in picks {
            match raster(feed, get, &radar, t).await {
                Ok(raster) => layers.push((radar, raster)),
                Err(e) => inpe_log(format_args!("{} @ {t}: {e}", radar.layer)),
            }
        }
        // Keep only what the next older slot or future live slots can use.
        feed.rasters.retain(|(_, t), _| {
            (*t > slot - SLOT_MS - WINDOW_MS && *t <= slot - SLOT_MS)
                || *t > newest + SLOT_MS - WINDOW_MS
        });
        if layers.is_empty() {
            // Late radars may still fill the newest slot; older ones are final.
            if slot < newest {
                feed.done.insert(slot);
            }
            continue;
        }
        let contributors = layers.len();
        let (frame, texture) = match build(layers, slot, work, palette, bounds).await {
            Ok(built) => built,
            Err(e) => {
                inpe_log(e);
                continue;
            }
        };
        inpe_log(format_args!(
            "{}: {contributors} radars, {} KiB",
            frame.scan_time,
            texture.len() >> 10
        ));
        let event = if feed.newest_sent.is_none_or(|n| slot > n) {
            feed.newest_sent = Some(slot);
            GridEvent::Frame {
                source_id: ID.into(),
                frame: Box::new(frame),
                texture,
                start_ms: slot,
            }
        } else {
            GridEvent::Backfill {
                source_id: ID.into(),
                frame: Box::new(frame),
                texture,
                start_ms: slot,
            }
        };
        if events.send(event).await.is_err() {
            return Ok(Outcome::Closed);
        }
        feed.done.insert(slot);
        published += 1;
    }
    Ok(Outcome::Published(published))
}

#[allow(clippy::too_many_arguments)]
async fn poll_loop<G, GF, N>(
    events: GridSender,
    retained: Vec<i64>,
    get: G,
    work: Arc<Semaphore>,
    palette: Arc<Vec<String>>,
    bounds: Arc<Vec<f64>>,
    interval: Duration,
    now: N,
) where
    G: Fn(String, usize) -> GF,
    GF: Future<Output = Result<Vec<u8>, String>>,
    N: Fn() -> i64,
{
    let mut feed = Feed {
        done: retained.iter().copied().collect(),
        newest_sent: retained.iter().copied().max(),
        ..Feed::default()
    };
    let mut offline = false;
    loop {
        let result = cycle(&mut feed, now(), &get, &events, &work, &palette, &bounds).await;
        let status = match result {
            Ok(Outcome::Closed) => return,
            Ok(Outcome::Published(_)) if feed.newest_sent.is_none() => Some(GridEvent::Silent {
                source_id: ID.into(),
                reason: "INPE lists no recent CAPPI 3 km radar frames".into(),
            }),
            Ok(Outcome::Published(_)) if offline => {
                offline = false;
                Some(GridEvent::Online {
                    source_id: ID.into(),
                })
            }
            Ok(Outcome::Published(_)) => None,
            Err(reason) => {
                offline = true;
                Some(GridEvent::Offline {
                    source_id: ID.into(),
                    reason,
                })
            }
        };
        if let Some(event) = status
            && events.send(event).await.is_err()
        {
            return;
        }
        if events.is_closed() {
            return;
        }
        sleep(interval).await;
    }
}

async fn get_http(path: String, max: usize) -> Result<Vec<u8>, String> {
    let url = format!("{HOST}{path}");
    let response = live_index::http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetching INPE {path}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("fetching INPE {path}: {e}"))?;
    live_index::take_body(response, max)
        .await
        .map_err(|e| format!("reading INPE {path}: {e}"))
}

#[cfg(test)]
mod tests;
