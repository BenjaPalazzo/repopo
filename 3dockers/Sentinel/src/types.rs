use std::collections::HashMap;

use serde::Deserialize;

// ── burst_list.json ───────────────────────────────────────────────────────────
//
// Structure (as written by the API server / local2safe convention):
// {
//   "<SLC_GRANULE>": {
//     "<SUBSWATH>": {
//       "<POL>": {
//         "<BURST_INDEX>": {
//           "DATA":     "/archive/…/N.tiff",
//           "METADATA": "/archive/…/N.xml"
//         }
//       }
//     }
//   }
// }

/// Per-burst file paths.
#[derive(Deserialize, Debug, Clone)]
pub struct BurstEntry {
    #[serde(rename = "DATA")]
    pub data: String,
    #[serde(rename = "METADATA")]
    pub metadata: String,
}

/// burst_list.json: SLC → Subswath → Polarisation → BurstIndex → BurstEntry
pub type BurstList = HashMap<String, HashMap<String, HashMap<String, HashMap<String, BurstEntry>>>>;

// ── specification.toml ────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
pub struct Spec {
    pub folders: Folders,
    #[serde(rename = "dem")]
    pub dem: DemSection,
    pub isce: IsceParams,
    // [processing.bounds] and [mintpy] are present but unused by this container.
}

#[derive(Deserialize, Debug)]
pub struct Folders {
    pub home: String,
    pub data: String,
    pub orbits: String,
    /// Path to the DEM *file* (e.g. /job/dem/full_res.dem.wgs84), not the directory.
    pub dem: String,
    pub aux: String,
    pub weather: String,
}

#[derive(Deserialize, Debug)]
pub struct DemSection {
    pub bounds: Bounds,
}

#[derive(Deserialize, Debug)]
pub struct Bounds {
    pub south: f64,
    pub north: f64,
    pub west: f64,
    pub east: f64,
}

#[derive(Deserialize, Debug)]
pub struct IsceParams {
    pub range_looks: u32,
    pub azimuth_looks: u32,
    pub connections: u32,
    #[serde(rename = "virtual")]
    pub use_virtual: bool,
}
