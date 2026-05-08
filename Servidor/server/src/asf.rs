//! ASF (Alaska Satellite Facility) search integration.
//!
//! Queries the ASF GeoJSON search API, selects the optimal Sentinel-1 path,
//! applies the adjacent-burst fallback, and produces:
//! - The DEM bounding box (from the union of all selected burst footprints).
//! - The `burst_list.json` structure consumed by the download container.

use std::collections::{HashMap, HashSet};
use url::Url;
use geo::{Area, BooleanOps, BoundingRect, Coord, LineString, MultiPolygon, Polygon, Rect};
use serde::{Deserialize, Serialize};

use shared::types::{BoundingBox, FlightPathError, JobRequest, Sensor};

// =============================================================================
// ASF GeoJSON response types
// =============================================================================

#[derive(Deserialize, Debug)]
pub struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize, Debug)]
struct Feature {
    geometry:   Geometry,
    properties: Properties,
}

#[derive(Deserialize, Debug)]
struct Geometry {
    /// Outer ring of the burst polygon. Each element is [lon, lat].
    coordinates: Vec<Vec<[f64; 2]>>,
}

#[derive(Deserialize, Debug, Clone)]
struct Properties {
    #[serde(rename = "pathNumber")]
    path_number: usize,

    #[serde(rename = "url")]
    tiff_url: String,

    #[serde(rename = "additionalUrls")]
    metadata_url: [String; 1],

    /// ISO-8601 acquisition start time; first 10 chars give the date.
    #[serde(rename = "startTime")]
    start_time: String,

    burst: BurstProperties,

    /// Scene (granule) identifier, e.g.
    /// `S1C_IW_SLC__1SDV_20251014T094805_…`.
    #[serde(rename = "sceneName")]
    scene_name: String,
}

#[derive(Deserialize, Debug, Clone)]
struct BurstProperties {
    #[serde(rename = "burstIndex")]
    burst_index: i64,

    subswath: String,
}

// =============================================================================
// Per-path accumulator (used during path selection)
// =============================================================================

struct PathAccum {
    features:     Vec<(Properties, Polygon)>,
    dates:        HashSet<String>,
    /// Unique (burst_id, subswath) pairs; used for burst-count heuristic.
    seen_bursts:  HashSet<(i64, String)>,
    union:        MultiPolygon,
    coverage:     f64,
}

// =============================================================================
// Output types
// =============================================================================

/// The information derived from a successful ASF search, ready to be written
/// to disk and stored in the job record.
pub struct AsfSearchResult {
    /// Bounding box of the union of all selected burst footprints.
    /// Used for `[dem.bounds]` in `specification.toml`.
    pub dem_bbox: Rect,

    /// The orbit path number chosen.
    pub path: u32,

    /// Burst list ready to be serialised to `burst_list.json`.
    /// Structure: granule → subswath → polarization → burst_index → paths.
    pub burst_list: BurstList,

    /// Per-acquisition burst lists ready to be written to `burst_stitch/`.
    /// Structure: date (YYYY-mm-dd) → granule → subswath → polarization → burst_index → paths.
    pub burst_stitch: BurstStitchMap,
}

/// `burst_list.json` top-level structure.
/// granule_name → subswath → polarization → burst_index → { DATA, METADATA }
pub type BurstList = HashMap<String, HashMap<String, HashMap<String, HashMap<String, BurstPaths>>>>;

/// `burst_stitch/{YYYY-mm-dd}.json` top-level structure.
/// date → granule_name → subswath → polarization → burst_index → { DATA, METADATA }
pub type BurstStitchMap = HashMap<String, BurstList>;

#[derive(Serialize, Debug, Clone)]
pub struct BurstPaths {
    #[serde(rename = "DATA")]
    pub data: String,
    #[serde(rename = "METADATA")]
    pub metadata: String,
}

// =============================================================================
// Public entry point
// =============================================================================

/// Queries ASF, selects the optimal path, applies the adjacent-burst fallback,
/// and returns an `AsfSearchResult`.
///
/// # Errors
/// - `FlightPathError::InsufficientMaterial` — no images found.
/// - `FlightPathError::InvalidPath` — an explicit path was requested but has
///   insufficient coverage.
pub async fn search(
    req: &JobRequest,
    asf_endpoint: &str,
) -> Result<AsfSearchResult, FlightPathError> {
    let response = query_asf(req, asf_endpoint).await;
    let fc: FeatureCollection =
        serde_json::from_str(&response).map_err(|_| FlightPathError::InsufficientMaterial)?;

    if fc.features.is_empty() {
        return Err(FlightPathError::InsufficientMaterial);
    }

    // Build AOI polygon for coverage computation.
    let aoi: Polygon = BoundingBox::from_bounds(req.north, req.south, req.east, req.west)
        .map(Into::into)
        .unwrap_or_else(|_| {
            // Already validated upstream; this path should not be reached.
            panic!("invalid bounding box reached asf::search")
        });
    let ref_area = aoi.unsigned_area();

    // Accumulate features per path.
    let mut paths: HashMap<usize, PathAccum> = HashMap::new();

    for feature in &fc.features {
        let path = feature.properties.path_number;
        let date = &feature.properties.start_time[..10];
        let burst_key = (
            feature.properties.burst.burst_index,
            feature.properties.burst.subswath.clone(),
        );

        let ring: Vec<Coord> = feature.geometry.coordinates[0]
            .iter()
            .map(|p| Coord { x: p[0], y: p[1] })
            .collect();
        let polygon = Polygon::new(LineString(ring), vec![]);

        let accum = paths.entry(path).or_insert_with(|| PathAccum {
            features:    Vec::new(),
            dates:       HashSet::new(),
            seen_bursts: HashSet::new(),
            union:       MultiPolygon(vec![]),
            coverage:    0.0,
        });

        accum.dates.insert(date.to_string());

        if accum.seen_bursts.insert(burst_key) {
            let new_poly = MultiPolygon(vec![polygon.clone()]);
            accum.union = accum.union.union(&new_poly);
        }

        accum.features.push((feature.properties.clone(), polygon));
    }

    // Compute coverage for each path.
    for accum in paths.values_mut() {
        let intersection = aoi.intersection(&accum.union);
        accum.coverage = (intersection.unsigned_area() / ref_area).clamp(0.0, 1.0);
    }

    // If the caller specified a path, restrict to that path only.
    let chosen_path = if let Some(explicit_path) = req.path {
        let path_num = explicit_path as usize;
        if !paths.contains_key(&path_num) {
            return Err(FlightPathError::InvalidPath);
        }
        path_num
    } else {
        select_best_path(&paths)?
    };

    let accum = paths.remove(&chosen_path).unwrap();

    // Verify coverage is meaningful for an explicit path.
    if req.path.is_some() && accum.coverage < 0.01 {
        return Err(FlightPathError::InvalidPath);
    }

    // Collect selected features (filtered to chosen path).
    let selected_features: Vec<(Properties, Polygon)> = accum.features;

    // Apply adjacent-burst fallback if needed.
    let final_features = apply_adjacent_burst_fallback(selected_features, &fc.features, chosen_path);

    // Compute DEM bounding box from union of selected burst footprints.
    let union = final_features
        .iter()
        .fold(MultiPolygon(vec![]), |acc, (_, poly)| {
            acc.union(&MultiPolygon(vec![poly.clone()]))
        });

    let dem_bbox = union
        .bounding_rect()
        .ok_or(FlightPathError::InsufficientMaterial)?;

    // Build burst list.
    let burst_list = build_burst_list(&final_features);
    let burst_stitch = build_burst_stitch(&final_features);

    Ok(AsfSearchResult {
        dem_bbox,
        path: chosen_path as u32,
        burst_list,
        burst_stitch,
    })
}

// =============================================================================
// Path selection
// =============================================================================

/// Selects the best path using the priority chain:
/// 1. Most AOI coverage (descending).
/// 2. Most acquisition dates (descending).
/// 3. Fewest bursts per acquisition > 1 (ascending) — storage optimisation.
/// 4. Smallest path number (ascending) — tiebreaker.
fn select_best_path(paths: &HashMap<usize, PathAccum>) -> Result<usize, FlightPathError> {
    if paths.is_empty() {
        return Err(FlightPathError::InsufficientMaterial);
    }

    let best = paths
        .iter()
        .min_by(|(num_a, a), (num_b, b)| {
            // All comparisons inverted — we use min_by to pick the "best".
            b.coverage
                .partial_cmp(&a.coverage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.dates.len().cmp(&a.dates.len()))
                .then_with(|| {
                    // Bursts per acquisition for a > 1; fewer is better.
                    let bpa_a = bursts_per_acquisition_gt1(a);
                    let bpa_b = bursts_per_acquisition_gt1(b);
                    bpa_a.cmp(&bpa_b)
                })
                .then_with(|| num_a.cmp(num_b))
        })
        .unwrap();

    Ok(*best.0)
}

/// Returns the number of unique burst (id, subswath) pairs divided by the
/// number of unique dates, when that quotient is > 1.  Returns 0 if there is
/// only 1 burst per acquisition (which then triggers the fallback).
fn bursts_per_acquisition_gt1(accum: &PathAccum) -> usize {
    if accum.dates.is_empty() {
        return 0;
    }
    let bpa = accum.seen_bursts.len() / accum.dates.len();
    if bpa > 1 { bpa } else { 0 }
}

// =============================================================================
// Adjacent-burst fallback
// =============================================================================

/// If the selected features contain only one unique burst per acquisition,
/// adds an adjacent burst from the same subswath by decrementing the burst
/// index (or incrementing if the index is 0).
///
/// The adjacent burst is looked up from the full feature collection returned
/// by ASF to ensure it exists in the database.
fn apply_adjacent_burst_fallback(
    mut selected: Vec<(Properties, Polygon)>,
    all_features: &[Feature],
    path: usize,
) -> Vec<(Properties, Polygon)> {
    // Count unique burst (id, subswath) pairs.
    let unique_bursts: HashSet<(i64, String)> = selected
        .iter()
        .map(|(p, _)| (p.burst.burst_index, p.burst.subswath.clone()))
        .collect();

    let unique_dates: HashSet<&str> = selected
        .iter()
        .map(|(p, _)| &p.start_time[..10])
        .collect();

    // If there's already more than one burst per acquisition, no fallback needed.
    if unique_bursts.len() > unique_dates.len().max(1) {
        return selected;
    }

    // Find the single burst key (we expect exactly one unique burst id+subswath).
    if let Some((burst_id, subswath)) = unique_bursts.into_iter().next() {
        let adjacent_id = if burst_id == 0 {
            burst_id + 1
        } else {
            burst_id - 1
        };

        // Look up matching features in the full collection.
        let adjacent: Vec<(Properties, Polygon)> = all_features
            .iter()
            .filter(|f| {
                f.properties.path_number == path
                    && f.properties.burst.burst_index == adjacent_id
                    && f.properties.burst.subswath == subswath
            })
            .map(|f| {
                let ring: Vec<Coord> = f.geometry.coordinates[0]
                    .iter()
                    .map(|p| Coord { x: p[0], y: p[1] })
                    .collect();
                let poly = Polygon::new(LineString(ring), vec![]);
                (f.properties.clone(), poly)
            })
            .collect();

        if adjacent.is_empty() {
            tracing::warn!(
                burst_id,
                adjacent_id,
                subswath,
                "adjacent burst not found in ASF response; proceeding with single burst"
            );
        } else {
            tracing::info!(
                burst_id,
                adjacent_id,
                subswath,
                "adjacent burst fallback applied"
            );
            selected.extend(adjacent);
        }
    }

    selected
}

// =============================================================================
// Burst list builder
// =============================================================================

/// Builds the `burst_list.json` structure from the selected features.
///
/// Paths are relative to `/archive` — the mount point used inside download
/// containers — rather than the host-side `archive_root`:
/// `/{scene_name}/{subswath}/{polarization}/{burst_index}.{ext}`
fn build_burst_list(features: &[(Properties, Polygon)]) -> BurstList {
    let mut burst_list: BurstList = HashMap::new();

    for (props, _) in features {

        let scene_name = Url::parse(&props.tiff_url).unwrap();
        let scene_name: Vec<_> = scene_name.path_segments().unwrap().collect();
        let subswath    = props.burst.subswath.to_uppercase();
        let burst_index = props.burst.burst_index.to_string();
        let polarization = "VV".to_string();
        // nota mental que cuando lo pasemos a linux, borrar eso, osea corregir eso
        // Paths are relative to the /archive container mount point.
        let rel_base = std::path::Path::new("/archive")
            .join(scene_name[0].to_string())
            .join(&subswath)
            .join(&polarization)
            .join(&burst_index);


        let data_path     = rel_base.with_extension("tiff").to_string_lossy().into_owned();
        let metadata_path = rel_base.with_extension("xml").to_string_lossy().into_owned();

        burst_list
            .entry(scene_name[0].to_string())
            .or_default()
            .entry(subswath)
            .or_default()
            .entry(polarization)
            .or_default()
            .insert(burst_index, BurstPaths { data: data_path, metadata: metadata_path });
    }

    burst_list
}

/// Splits the selected features into per-acquisition `BurstList` trees, keyed
/// by acquisition date (`YYYY-mm-dd` derived from `start_time`).
///
/// Each entry is written to `burst_stitch/{date}.json` by
/// `specification::write_job_files`.  Paths follow the same `/archive`-relative
/// convention as `build_burst_list`.
fn build_burst_stitch(features: &[(Properties, Polygon)]) -> BurstStitchMap {
    let mut stitch: BurstStitchMap = HashMap::new();

    for (props, _) in features {
        // start_time is ISO-8601; the first 10 characters are "YYYY-mm-dd".
        let date = props.start_time[..10].to_string();
        let scene_name = Url::parse(&props.tiff_url).unwrap();
        let scene_name: Vec<_> = scene_name.path_segments().unwrap().collect();
        let subswath     = props.burst.subswath.to_uppercase();
        let burst_index  = props.burst.burst_index.to_string();
        let polarization = "VV".to_string();

        let rel_base = std::path::Path::new("/archive")
            .join(scene_name[0].to_string())
            .join(&subswath)
            .join(&polarization)
            .join(&burst_index);


        let data_path     = rel_base.with_extension("tiff").to_string_lossy().into_owned();
        let metadata_path = rel_base.with_extension("xml").to_string_lossy().into_owned();

        stitch
            .entry(date)
            .or_default()
            .entry(scene_name[0].to_string())
            .or_default()
            .entry(subswath)
            .or_default()
            .entry(polarization)
            .or_default()
            .insert(burst_index, BurstPaths { data: data_path, metadata: metadata_path });
    }

    stitch
}

// =============================================================================
// ASF HTTP query
// =============================================================================

async fn query_asf(req: &JobRequest, endpoint: &str) -> String {
    let dataset = req
        .sensor
        .unwrap_or_default()
        .asf_dataset();

    let bounds = format!(
        "intersectsWith=polygon%28%28{west}+{north},{east}+{north},{east}+{south},{west}+{south},{west}+{north}%29%29",
        north = req.north,
        south = req.south,
        east  = req.east,
        west  = req.west,
    );

    let url = format!(
        "{endpoint}?dataset={dataset}&polarization=VV&{bounds}&start={start}&end={end}&output=geojson",
        start = req.start,
        end   = req.end,
    );

    tracing::debug!(%url, "querying ASF");

    reqwest::get(&url)
        .await
        .expect("ASF request failed")
        .text()
        .await
        .expect("ASF response read failed")
}
