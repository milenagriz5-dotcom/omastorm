//! Offline INPE tests: every input is synthesized (\`docs/inpe-brazil.md\`).

use super::*;
use crate::{
    protocol::{AdapterTarget, FrameWire, GeoPoint},
    source::SourceRegistry,
};
use std::sync::Mutex;
use tokio::sync::mpsc;

const CHAPECO: &str = "sdcsc~chapeco~cappi~cappi3km";

fn chapeco(latest_ms: i64) -> Radar {
    Radar {
        layer: CHAPECO.into(),
        south: -29.281,
        west: -55.172,
        north: -24.777,
        east: -50.035,
        latest_ms,
    }
}

fn ms(text: &str) -> i64 {
    parse_time(text).unwrap()
}

fn legend_json() -> String {
    let entries: Vec<String> = LEGEND
        .iter()
        .map(|(v, hex)| format!(r##"{{"label":"{v}","color":"#{hex:06x}","value":{v:.1}}}"##))
        .collect();
    format!(r#"{{"title":"CAPPI","entries":[{}]}}"#, entries.join(","))
}

fn rgba_png(width: u32, height: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&fill(x, y));
        }
    }
    sweep::png(width, height, &pixels).unwrap()
}

fn colour_of(dbz: usize) -> [u8; 4] {
    let [r, g, b] = rgb(LEGEND[dbz].1).map(|c| c as u8);
    [r, g, b, 255]
}

#[test]
fn legend_colours_invert_to_their_values_and_blends_interpolate() {
    for (i, &(value, hex)) in LEGEND.iter().enumerate() {
        let first = LEGEND.iter().position(|&(_, h)| h == hex).unwrap();
        let expected = LEGEND[first].0;
        let pixel = rgb(hex).map(|c| c as u8);
        assert_eq!(invert(pixel), Some(expected), "entry {i} ({value} dBZ)");
    }
    // A resampled pixel half way between 10 (#00f3f8) and 11 (#00d8ec).
    let blended = invert([0x00, 0xe6, 0xf2]).unwrap();
    assert!((blended - 10.5).abs() < 0.1, "{blended}");
    // Nothing on the ramp is near black or saturated magenta-green.
    assert_eq!(invert([0, 0, 0]), None);
    assert_eq!(invert([255, 0, 255]), None);
    assert!(legend_matches(legend_json().as_bytes()));
    let changed = legend_json().replace("#00f3f8", "#00f3f9");
    assert!(!legend_matches(changed.as_bytes()));
    assert!(!legend_matches(b"{\"entries\":[]}"));
    assert!(!legend_matches(b"not json"));
}

#[test]
fn measured_codes_floor_to_half_dbz() {
    assert_eq!(measured_code(0.0), 66);
    assert_eq!(measured_code(5.0), 76);
    assert_eq!(measured_code(4.99), 75);
    assert_eq!(measured_code(10.5), 87);
    assert_eq!(measured_code(-100.0), 2);
    assert_eq!(measured_code(500.0), 255);
}

#[test]
fn decode_keeps_transparent_as_nothing_and_rejects_foreign_palettes() {
    let png = rgba_png(4, 2, |x, y| match (x, y) {
        (0, 0) => [0, 0, 0, 0],
        (1, 0) => [255, 255, 255, 255],
        (2, 0) => colour_of(35),
        (3, 0) => [0x00, 0xe6, 0xf2, 255],
        _ => [0, 0, 0, 0],
    });
    let raster = decode_radar(&png).unwrap();
    assert_eq!((raster.width, raster.height), (4, 2));
    let blend = measured_code(invert([0x00, 0xe6, 0xf2]).unwrap());
    assert_eq!(blend, 86, "10.49 dBZ floors to the 10.0 half-step");
    assert_eq!(&raster.codes[..4], &[0, 66, measured_code(35.0), blend]);
    assert!(raster.codes[4..].iter().all(|&c| c == 0));

    let foreign = rgba_png(10, 10, |x, _| {
        if x < 5 {
            [255, 0, 255, 255]
        } else {
            colour_of(20)
        }
    });
    let error = decode_radar(&foreign).unwrap_err();
    assert!(error.contains("match no legend colour"), "{error}");
}

#[test]
fn catalog_keeps_live_cappi_layers_inside_the_grid() {
    let now = ms("2026-11-15T23:30:00Z");
    let body = r#"{"sources":[
      {"slug":"modelos","products":[{"layer":"x~y~cappi~cappi3km",
        "bounds":[[-29,-55],[-25,-50]],"latest_time":"2026-11-15T23:00:00Z"}]},
      {"slug":"radar","products":[
        {"layer":"sdcsc~chapeco~cappi~cappi3km","bounds":[[-29.281,-55.172],[-24.777,-50.035]],
         "latest_time":"2026-11-15T23:00:00Z","cadence_minutes":6},
        {"layer":"sdcsc~chapeco~ppi~ppicz","bounds":[[-29.281,-55.172],[-24.777,-50.035]],
         "latest_time":"2026-11-15T23:00:00Z"},
        {"layer":"cemaden~tresmarias~cappi~cappi3km","bounds":[[-20.4,-47.8],[-15.9,-43.0]],
         "latest_time":"2018-05-10T11:50:00Z"},
        {"layer":"far~away~cappi~cappi3km","bounds":[[40,-10],[45,-5]],
         "latest_time":"2026-11-15T23:00:00Z"},
        {"layer":"Bad/Layer~cappi~cappi3km","bounds":[[-29,-55],[-25,-50]],
         "latest_time":"2026-11-15T23:00:00Z"},
        {"layer":"undefined~undefined","bounds":null,"latest_time":null}
      ]}]}"#;
    let radars = parse_catalog(body.as_bytes(), now).unwrap();
    assert_eq!(radars, vec![chapeco(ms("2026-11-15T23:00:00Z"))]);
    assert!(parse_catalog(br#"{"sources":[]}"#, now).is_err());
    assert!(parse_catalog(b"<html>", now).is_err());
}

#[test]
fn times_parse_whole_minutes_in_order() {
    let body = br#"{"times":["2026-11-15T23:12:00Z","2026-11-15T22:48:00Z",
        "2026-11-15T23:12:00Z","2026-11-15T23:13:30Z","garbage"]}"#;
    assert_eq!(
        parse_times(body).unwrap(),
        vec![ms("2026-11-15T22:48:00Z"), ms("2026-11-15T23:12:00Z")]
    );
}

#[test]
fn slots_wait_for_delivery_and_pick_the_newest_frame_in_the_window() {
    assert_eq!(
        newest_slot(ms("2026-11-15T23:31:20Z")),
        ms("2026-11-15T23:10:00Z")
    );
    assert_eq!(
        newest_slot(ms("2026-11-15T23:30:00Z")),
        ms("2026-11-15T23:10:00Z")
    );
    let slot = ms("2026-11-15T23:10:00Z");
    let times = [
        ms("2026-11-15T22:50:00Z"),
        ms("2026-11-15T22:54:00Z"),
        ms("2026-11-15T23:06:00Z"),
        ms("2026-11-15T23:12:00Z"),
    ];
    assert_eq!(pick(&times, slot), Some(ms("2026-11-15T23:06:00Z")));
    assert_eq!(
        pick(&times[..1], slot),
        None,
        "window excludes slot - 20 min"
    );
    assert_eq!(pick(&times[..2], slot), Some(ms("2026-11-15T22:54:00Z")));
    assert_eq!(
        pick(&times[3..], slot),
        None,
        "never a frame after the slot"
    );
}

fn cell_of(lat: f64, lon: f64) -> usize {
    let col = ((lon - GEOTRANSFORM[0]) / GEOTRANSFORM[1]) as usize;
    let row = ((lat - GEOTRANSFORM[3]) / GEOTRANSFORM[5]) as usize;
    row * WIDTH as usize + col
}

#[test]
fn rows_are_placed_linearly_in_web_mercator_like_leaflet() {
    let radar = chapeco(0);
    let (w, h) = (500usize, 510usize);
    // One echo row exactly half way down the image.
    let mut codes = vec![0u8; w * h];
    codes[(h / 2) * w..(h / 2 + 1) * w].fill(measured_code(40.0));
    let raster = RadarRaster {
        width: w,
        height: h,
        codes,
    };
    let cells = composite(&[(&radar, &raster)]);
    let lon = (radar.west + radar.east) / 2.0;
    let col = ((lon - GEOTRANSFORM[0]) / GEOTRANSFORM[1]) as usize;
    let hits: Vec<f64> = (0..HEIGHT as usize)
        .filter(|row| cells[row * WIDTH as usize + col] >= 2)
        .map(|row| GEOTRANSFORM[3] + (row as f64 + 0.5) * GEOTRANSFORM[5])
        .collect();
    assert!(!hits.is_empty());
    let mid = (mercator(radar.north) + mercator(radar.south)) / 2.0;
    let mercator_mid = (2.0 * mid.exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    let linear_mid = (radar.north + radar.south) / 2.0;
    for lat in &hits {
        assert!(
            (lat - mercator_mid).abs() <= STEP_DEG,
            "{lat} vs {mercator_mid}"
        );
        assert!(
            (lat - linear_mid).abs() > STEP_DEG,
            "linear placement would be wrong"
        );
    }
}

#[test]
fn coverage_is_the_range_circle_and_strongest_return_wins() {
    let a = chapeco(0);
    // A second radar 1.5° east, overlapping the first.
    let b = Radar {
        layer: "b~b~cappi~cappi3km".into(),
        west: a.west + 1.5,
        east: a.east + 1.5,
        ..a.clone()
    };
    let (w, h) = (100usize, 100usize);
    let blank = RadarRaster {
        width: w,
        height: h,
        codes: vec![0; w * h],
    };
    let strong = RadarRaster {
        width: w,
        height: h,
        codes: vec![measured_code(50.0); w * h],
    };
    let weak = RadarRaster {
        width: w,
        height: h,
        codes: vec![measured_code(20.0); w * h],
    };
    let (lat_c, lon_c) = a.centre();
    // Only radar A, nothing measured: inside range is undetect, the box
    // corner beyond range and everything else is missing.
    let cells = composite(&[(&a, &blank)]);
    assert_eq!(cells[cell_of(lat_c, lon_c)], UNDETECT);
    assert_eq!(cells[cell_of(a.north - 0.05, a.west + 0.05)], MISSING);
    assert_eq!(cells[cell_of(-10.0, -45.0)], MISSING);
    // Overlap: the stronger code wins whichever order radars arrive in, and
    // undetect from one radar never hides another's echo.
    let overlap = (lat_c, lon_c + 0.75);
    for layers in [[(&a, &weak), (&b, &strong)], [(&b, &strong), (&a, &weak)]] {
        let cells = composite(&layers);
        assert_eq!(cells[cell_of(overlap.0, overlap.1)], measured_code(50.0));
    }
    let cells = composite(&[(&a, &weak), (&b, &blank)]);
    assert_eq!(cells[cell_of(overlap.0, overlap.1)], measured_code(20.0));
}

#[test]
fn texture_classifies_codes_with_the_shared_palette() {
    let inpe = Inpe::new();
    let cells = [MISSING, UNDETECT, measured_code(4.0), measured_code(35.0)];
    let png = encode(&cells, 4, 1, &inpe.bounds, inpe.palette.len()).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut decoded).unwrap();
    assert_eq!(&decoded[0..4], &[0, 1, 0, 255]);
    assert_eq!(&decoded[4..8], &[0, 2, 0, 255]);
    let classes = inpe.palette.len();
    assert_eq!(
        &decoded[8..12],
        &[class_of(4.0, &inpe.bounds, classes) + 1, 0, 74, 255]
    );
    assert_eq!(
        &decoded[12..16],
        &[class_of(35.0, &inpe.bounds, classes) + 1, 0, 136, 255]
    );
    assert!(encode(&cells, 3, 1, &inpe.bounds, classes).is_err());
}

#[test]
fn grid_matches_the_envelope_and_stays_under_the_texture_check() {
    let east = GEOTRANSFORM[0] + f64::from(WIDTH) * GEOTRANSFORM[1];
    let south = GEOTRANSFORM[3] + f64::from(HEIGHT) * GEOTRANSFORM[5];
    assert!((east - INPE_BRAZIL.east).abs() < 1e-9, "{east}");
    assert!((south - INPE_BRAZIL.south).abs() < 1e-9, "{south}");
    let inpe = Inpe::new();
    let frame = frame(ms("2026-11-15T23:10:00Z"), &inpe.palette, &inpe.bounds);
    assert_eq!(frame.id, "inpe-brazil-20261115T231000Z");
    assert_eq!(frame.scan_time, "2026-11-15T23:10:00Z");
    let wire = serde_json::to_value(FrameWire::Mosaic(frame)).unwrap();
    assert_eq!(wire["kind"], "mosaic");
    assert_eq!(wire["crs"]["kind"], "geographic");
    assert_eq!(
        wire["measuredCode"],
        serde_json::json!({"scale": 2.0, "offset": 66.0})
    );
    assert!(wire.get("scale").is_none() && wire.get("elevationDeg").is_none());
    let (placeholder, png) = inpe.loading_placeholder().unwrap();
    assert_eq!(placeholder.status, FrameStatus::Partial);
    assert!(!png.is_empty());
}

#[test]
fn brazil_follows_to_the_composite_and_nowhere_else_changes() {
    let registry = SourceRegistry::compiled();
    let at = |lat, lon| {
        registry
            .covering_selection(GeoPoint { lat, lon }, None)
            .map(|s| s.source_id)
    };
    let foz = at(-25.548, -54.588).unwrap();
    assert_eq!(foz, ID);
    assert_eq!(at(-23.55, -46.63).as_deref(), Some(ID), "São Paulo");
    assert_eq!(at(-3.12, -60.02).as_deref(), Some(ID), "Manaus");
    assert_eq!(at(-34.6, -58.4), None, "Buenos Aires is outside the grid");
    assert_eq!(at(35.333, -97.278).as_deref(), Some("nexrad"));
    assert_eq!(at(51.5, -0.1).as_deref(), Some("opera"));
    let picked = registry
        .covering_selection(
            GeoPoint {
                lat: -25.5,
                lon: -54.6,
            },
            None,
        )
        .unwrap();
    assert_eq!(picked.target, AdapterTarget::Mosaic);
    assert_eq!(registry.history_policy(ID), Some(HISTORY));
    assert!(registry.polls(ID));
    let info = registry
        .hello_sources()
        .into_iter()
        .find(|s| s.id == ID)
        .unwrap();
    assert_eq!(info.attribution, ATTRIBUTION);
    assert!(crate::envelope::in_live_envelope(-54.588, -25.548));
}

/// A tiny Chapecó frame: transparent, with a 40 dBZ block in the middle.
fn frame_png() -> Vec<u8> {
    rgba_png(50, 51, |x, y| {
        if (20..30).contains(&x) && (20..30).contains(&y) {
            colour_of(40)
        } else {
            [0, 0, 0, 0]
        }
    })
}

struct Mock {
    catalog: String,
    times: String,
    legend: String,
    requests: Mutex<Vec<String>>,
}

impl Mock {
    fn new(latest: &str, times: &[&str]) -> Arc<Self> {
        let catalog = format!(
            r#"{{"sources":[{{"slug":"radar","products":[{{"layer":"{CHAPECO}",
            "bounds":[[-29.281,-55.172],[-24.777,-50.035]],"latest_time":"{latest}"}}]}}]}}"#
        );
        let times = format!(
            r#"{{"times":[{}]}}"#,
            times
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        Arc::new(Self {
            catalog,
            times,
            legend: legend_json(),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn get(
        self: &Arc<Self>,
    ) -> impl Fn(String, usize) -> std::future::Ready<Result<Vec<u8>, String>> + use<> {
        let mock = Arc::clone(self);
        move |path: String, _max| {
            mock.requests.lock().unwrap().push(path.clone());
            std::future::ready(Ok(if path == "catalog" {
                mock.catalog.clone().into_bytes()
            } else if path.ends_with("/times") {
                mock.times.clone().into_bytes()
            } else if path.ends_with("/legend") {
                mock.legend.clone().into_bytes()
            } else if path.starts_with("frames/radar/") && path.ends_with("/full") {
                frame_png()
            } else {
                return std::future::ready(Err(format!("unexpected {path}")));
            }))
        }
    }

    fn requests(&self) -> Vec<String> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_cycle_publishes_newest_live_then_history_and_never_refetches() {
    let mock = Mock::new(
        "2026-11-15T23:12:00Z",
        &[
            "2026-11-15T22:56:00Z",
            "2026-11-15T23:06:00Z",
            "2026-11-15T23:12:00Z",
        ],
    );
    let get = mock.get();
    let inpe = Inpe::new();
    let (palette, bounds) = (Arc::clone(&inpe.palette), Arc::clone(&inpe.bounds));
    let work = Arc::new(Semaphore::new(1));
    let (tx, mut rx) = mpsc::channel(32);
    let events = GridSender::new(tx, 7);
    let now = ms("2026-11-15T23:31:00Z"); // newest slot 23:10
    let mut feed = Feed::default();
    runtime().block_on(async {
        let outcome = cycle(&mut feed, now, &get, &events, &work, &palette, &bounds)
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Published(2)));
        let mut got = Vec::new();
        while let Ok(message) = rx.try_recv() {
            assert_eq!(message.session, 7);
            match message.event {
                GridEvent::Frame {
                    frame,
                    start_ms,
                    texture,
                    ..
                } => {
                    got.push(("live", frame.scan_time.clone(), start_ms));
                    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));
                    let reader = png::Decoder::new(Cursor::new(texture)).read_info().unwrap();
                    assert_eq!(reader.info().width, WIDTH);
                }
                GridEvent::Backfill {
                    frame, start_ms, ..
                } => got.push(("history", frame.scan_time.clone(), start_ms)),
                _ => panic!("unexpected event"),
            }
        }
        assert_eq!(
            got,
            vec![
                (
                    "live",
                    "2026-11-15T23:10:00Z".into(),
                    ms("2026-11-15T23:10:00Z")
                ),
                (
                    "history",
                    "2026-11-15T23:00:00Z".into(),
                    ms("2026-11-15T23:00:00Z")
                ),
            ]
        );
        let first = mock.requests();
        assert_eq!(first[0], "catalog");
        assert_eq!(
            first.iter().filter(|p| p.ends_with("/legend")).count(),
            1,
            "legend checked once per session"
        );
        assert!(first.contains(&format!("frames/radar/{CHAPECO}/202611152306/full")));
        assert!(first.contains(&format!("frames/radar/{CHAPECO}/202611152256/full")));
        assert!(
            !first.iter().any(|p| p.contains("202611152312")),
            "after the slot"
        );

        // Same catalog, same time: only the catalog is read again.
        let outcome = cycle(&mut feed, now, &get, &events, &work, &palette, &bounds)
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Published(0)));
        assert_eq!(mock.requests(), vec!["catalog".to_string()]);
        assert!(rx.try_recv().is_err());
    });
}

#[test]
fn a_changed_legend_skips_the_radar_and_the_feed_reports_silence() {
    let mock = Mock::new("2026-11-15T23:12:00Z", &["2026-11-15T23:06:00Z"]);
    let mock = Arc::new(Mock {
        catalog: mock.catalog.clone(),
        times: mock.times.clone(),
        legend: legend_json().replace("#ffff00", "#fefe00"),
        requests: Mutex::new(Vec::new()),
    });
    let get = mock.get();
    let inpe = Inpe::new();
    let (tx, mut rx) = mpsc::channel(8);
    let events = GridSender::new(tx, 1);
    runtime().block_on(async {
        let poll = tokio::spawn(poll_loop(
            events,
            Vec::new(),
            get,
            Arc::new(Semaphore::new(1)),
            Arc::clone(&inpe.palette),
            Arc::clone(&inpe.bounds),
            Duration::from_secs(3600),
            || ms("2026-11-15T23:31:00Z"),
        ));
        let message = timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(message.event, GridEvent::Silent { .. }));
        assert!(
            !mock.requests().iter().any(|p| p.starts_with("frames/")),
            "no frame is fetched for a radar with an unexpected legend"
        );
        poll.abort();
    });
}

#[test]
fn an_unreachable_catalog_is_offline_and_retained_slots_are_skipped() {
    let inpe = Inpe::new();
    let (tx, mut rx) = mpsc::channel(8);
    let events = GridSender::new(tx, 1);
    runtime().block_on(async {
        let poll = tokio::spawn(poll_loop(
            events.clone(),
            Vec::new(),
            |_path: String, _max| {
                std::future::ready(Err::<Vec<u8>, _>("connection refused".into()))
            },
            Arc::new(Semaphore::new(1)),
            Arc::clone(&inpe.palette),
            Arc::clone(&inpe.bounds),
            Duration::from_secs(3600),
            || ms("2026-11-15T23:31:00Z"),
        ));
        let message = timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(message.event, GridEvent::Offline { .. }));
        poll.abort();
    });

    // A restart that already holds 23:10 and 23:00 fetches nothing new.
    let mock = Mock::new(
        "2026-11-15T23:12:00Z",
        &["2026-11-15T22:56:00Z", "2026-11-15T23:06:00Z"],
    );
    let get = mock.get();
    let mut feed = Feed {
        done: [ms("2026-11-15T23:10:00Z"), ms("2026-11-15T23:00:00Z")].into(),
        newest_sent: Some(ms("2026-11-15T23:10:00Z")),
        ..Feed::default()
    };
    runtime().block_on(async {
        let outcome = cycle(
            &mut feed,
            ms("2026-11-15T23:31:00Z"),
            &get,
            &events,
            &Arc::new(Semaphore::new(1)),
            &Arc::clone(&inpe.palette),
            &Arc::clone(&inpe.bounds),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Outcome::Published(0)));
    });
    assert!(!mock.requests().iter().any(|p| p.starts_with("frames/")));
}
