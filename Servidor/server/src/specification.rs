//! Writes `specification.toml` and `burst_list.json` into a job's working
//! directory, and scaffolds the required subdirectory layout.

use std::path::Path;

use geo::Rect;
use serde::Serialize;

use crate::asf::{BurstList, BurstStitchMap};
use crate::error::AppError;

// =============================================================================
// TOML specification model
// =============================================================================

/// The full contents of `specification.toml` written by the server.
///
/// Serialised with `toml` and written verbatim; the download and ISCE2
/// containers read it directly.
#[derive(Serialize)]
struct Specification {
    folders: Folders,
    dem: DemSection,
    isce: IsceSection,
    processing: ProcessingSection,
    mintpy: MintpySection,
}

#[derive(Serialize)]
struct Folders {
    home: &'static str,
    data: &'static str,
    orbits: &'static str,
    dem: &'static str,
    aux: &'static str,
    weather: &'static str,
}

#[derive(Serialize)]
struct DemSection {
    bounds: Bounds,
}

#[derive(Serialize)]
struct Bounds {
    south: f64,
    north: f64,
    west: f64,
    east: f64,
}

#[derive(Serialize)]
struct IsceSection {
    range_looks: u32,
    azimuth_looks: u32,
    connections: u8,
    virt: bool,
}

#[derive(Serialize)]
struct ProcessingSection {
    bounds: Bounds,
}

#[derive(Serialize)]
struct MintpySection {
    tropo: &'static str,
}

// =============================================================================
// Parameters passed in from the submit pipeline
// =============================================================================

pub struct SpecificationParams {
    /// Bounding box of the burst union — used for `[dem.bounds]`.
    pub dem_bbox: Rect,
    /// AOI bounding box — used for `[processing.bounds]`.
    pub aoi_north: f64,
    pub aoi_south: f64,
    pub aoi_east: f64,
    pub aoi_west: f64,
    pub range_looks: u32,
    pub azimuth_looks: u32,
    pub connections: u8,
}

// =============================================================================
// Writer
// =============================================================================

/// Scaffolds the job working directory and writes `specification.toml`,
/// `burst_list.json`, and the per-acquisition files under `burst_stitch/`.
///
/// Directory layout created:
/// ```
/// {work_dir}/
/// ├── specification.toml
/// ├── burst_list.json
/// ├── burst_stitch/
/// │   ├── {YYYY-mm-dd}.json   (one file per acquisition date)
/// │   └── …
/// ├── data/
/// ├── dem/
/// ├── orbits/
/// ├── aux/
/// ├── weather/
/// └── results/
/// ```
pub fn write_job_files(
    work_dir: &Path,
    params: &SpecificationParams,
    burst_list: &BurstList,
    burst_stitch: &BurstStitchMap,
) -> Result<(), AppError> {
    // --- Scaffold directories ---
    for subdir in &["data", "dem", "orbits", "aux", "weather", "results", "burst_stitch", "series"] {
        std::fs::create_dir_all(work_dir.join(subdir))?;
    }

    // --- specification.toml ---
    let spec = Specification {
        folders: Folders {
            home: "/job",
            data: "/job/data",
            orbits: "/job/orbits",
            dem: "/job/dem/full_res.dem.wgs84",
            aux: "/job/aux",
            weather: "/job/weather",
        },
        dem: DemSection {
            bounds: Bounds {
                south: params.dem_bbox.min().y,
                north: params.dem_bbox.max().y,
                west: params.dem_bbox.min().x,
                east: params.dem_bbox.max().x,
            },
        },
        isce: IsceSection {
            range_looks: params.range_looks,
            azimuth_looks: params.azimuth_looks,
            connections: params.connections,
            virt: true,
        },
        processing: ProcessingSection {
            bounds: Bounds {
                south: params.aoi_south,
                north: params.aoi_north,
                west: params.aoi_west,
                east: params.aoi_east,
            },
        },
        mintpy: MintpySection {
            tropo: "height_correlation",
        },
    };

    let toml_str = toml::to_string_pretty(&spec)
        .map_err(|e| AppError::Internal(format!("serialising specification.toml: {e}")))?;

    std::fs::write(work_dir.join("specification.toml"), toml_str)?;

    // --- burst_list.json ---
    let json_str = serde_json::to_string_pretty(burst_list)
        .map_err(|e| AppError::Internal(format!("serialising burst_list.json: {e}")))?;

    std::fs::write(work_dir.join("burst_list.json"), json_str)?;

    // --- burst_stitch/{YYYY-mm-dd}.json ---
    let stitch_dir = work_dir.join("burst_stitch");
    for (date, date_burst_list) in burst_stitch {
        let json_str = serde_json::to_string_pretty(date_burst_list)
            .map_err(|e| AppError::Internal(format!("serialising burst_stitch/{date}.json: {e}")))?;

        std::fs::write(stitch_dir.join(format!("{date}.json")), json_str)?;
    }

    // --- series/mintpy_params.cfg ---
    // Paths are relative to /job as seen inside the container.
    let mintpy_cfg = "\
mintpy.load.processor        = isce\n\
mintpy.load.metaFile         = /job/reference/IW*.xml\n\
mintpy.load.baselineDir      = /job/baselines\n\
mintpy.load.unwFile          = /job/merged/interferograms/*/filt_*.unw\n\
mintpy.load.corFile          = /job/merged/interferograms/*/filt_*.cor\n\
mintpy.load.connCompFile     = /job/merged/interferograms/*/filt_*.unw.conncomp\n\
mintpy.load.demFile          = /job/merged/geom_reference/hgt.rdr\n\
mintpy.load.lookupYFile      = /job/merged/geom_reference/lat.rdr\n\
mintpy.load.lookupXFile      = /job/merged/geom_reference/lon.rdr\n\
mintpy.load.incAngleFile     = /job/merged/geom_reference/los.rdr\n\
mintpy.load.azAngleFile      = /job/merged/geom_reference/los.rdr\n\
mintpy.load.shadowMaskFile   = /job/merged/geom_reference/shadowMask.rdr\n\
mintpy.load.waterMaskFile    = /job/merged/geom_reference/waterMask.rdr\n\
mintpy.networkInversion.weightFunc   = var\n\
mintpy.networkInversion.maskDataset  = coherence\n\
mintpy.reference.lalo        = auto\n\
mintpy.subset.lalo           = auto\n\
mintpy.deramp                = linear\n\
mintpy.troposphericDelay.method      = pyaps\n\
mintpy.unwrapError.method    = auto\n\
mintpy.troposphericDelay.weatherDir  = /job/weather\n\
mintpy.networkInversion.maskThreshold = 0.3\n";

    std::fs::write(work_dir.join("series").join("mintpy_params.cfg"), mintpy_cfg)?;

    Ok(())
}
