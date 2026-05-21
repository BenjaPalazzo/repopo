//! Domain types shared between the API server and (where relevant) the
//! scheduler.  These were previously in an abandoned `common` crate; they now
//! live here so the whole workspace shares one definition.

use chrono::{Local, NaiveDate};
use geo::{polygon, Polygon};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// =============================================================================
// Coordinate newtypes
// =============================================================================

#[derive(Serialize, Deserialize, PartialEq, PartialOrd, Debug, Clone, Copy)]
pub struct Latitude(pub f64);

impl TryFrom<f64> for Latitude {
    type Error = CoordinatesError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if (-90.0..=90.0).contains(&value) {
            Ok(Latitude(value))
        } else {
            Err(CoordinatesError::OutOfBoundsLat(value))
        }
    }
}

impl Latitude {
    pub fn value(self) -> f64 {
        self.0
    }
}

#[derive(Serialize, Deserialize, PartialEq, PartialOrd, Debug, Clone, Copy)]
pub struct Longitude(pub f64);

impl TryFrom<f64> for Longitude {
    type Error = CoordinatesError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if (-180.0..=180.0).contains(&value) {
            Ok(Longitude(value))
        } else {
            Err(CoordinatesError::OutOfBoundsLon(value))
        }
    }
}

impl Longitude {
    pub fn value(self) -> f64 {
        self.0
    }
}

// =============================================================================
// BoundingBox
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BoundingBox {
    pub north: Latitude,
    pub south: Latitude,
    pub east:  Longitude,
    pub west:  Longitude,
}

impl BoundingBox {
    /// Validates and constructs a `BoundingBox` from raw `f64` bounds.
    pub fn from_bounds(
        north: f64,
        south: f64,
        east: f64,
        west: f64,
    ) -> Result<Self, BoundingBoxError> {
        let north = Latitude::try_from(north)?;
        let south = Latitude::try_from(south)?;
        let east  = Longitude::try_from(east)?;
        let west  = Longitude::try_from(west)?;

        if north <= south {
            return Err(BoundingBoxError::InvalidLatSpan {
                north: north.value(),
                south: south.value(),
            });
        }
        if east <= west {
            return Err(BoundingBoxError::InvalidLonSpan {
                east: east.value(),
                west: west.value(),
            });
        }

        Ok(BoundingBox { north, south, east, west })
    }

    /// Area in km² (approximate equirectangular).
    pub fn area_km2(&self) -> f64 {
        const DEG_TO_KM_LAT: f64 = 111.32;
        let mid_lat = ((self.north.0 + self.south.0) / 2.0).to_radians();
        let lat_km  = (self.north.0 - self.south.0).abs() * DEG_TO_KM_LAT;
        let lon_km  = (self.east.0  - self.west.0).abs()  * DEG_TO_KM_LAT * mid_lat.cos();
        lat_km * lon_km
    }
}

impl From<BoundingBox> for Polygon {
    fn from(bb: BoundingBox) -> Polygon {
        polygon!(
            exterior: [
                (x: bb.west.0, y: bb.north.0),
                (x: bb.west.0, y: bb.south.0),
                (x: bb.east.0, y: bb.south.0),
                (x: bb.east.0, y: bb.north.0),
            ],
            interiors: []
        )
    }
}

// =============================================================================
// TimeRange
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TimeRange {
    pub start: NaiveDate,
    pub end:   NaiveDate,
}

impl TimeRange {
    pub fn from_bounds(start: NaiveDate, end: NaiveDate) -> Result<Self, TimeRangeError> {
        let today = Local::now().date_naive();
        if start > today || end > today {
            return Err(TimeRangeError::FutureDate);
        }
        if start > end {
            return Err(TimeRangeError::InvertedBounds);
        }
        Ok(TimeRange { start, end })
    }

    pub fn duration_days(&self) -> i64 {
        (self.end - self.start).num_days()
    }
}

// =============================================================================
// Sensor
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sensor {
    S1Burst,
    Nisar,
}

impl Sensor {
    /// The dataset string expected by the ASF search API.
    pub fn asf_dataset(&self) -> &'static str {
        match self {
            Self::S1Burst => "SLC-BURST",
            Self::Nisar   => "NISAR",
        }
    }
}

impl Default for Sensor {
    fn default() -> Self {
        Self::S1Burst
    }
}

// =============================================================================
// ProcessingWorkflow
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingWorkflow {
    /// Small Baseline Subset: ISCE2 → MintPy.
    Sbas,
    /// Persistent Scatterer InSAR: ISCE2 → MintPy → MiaplPy.
    PsInsar,
    /// Time-series analysis: ISCE2 → MintPy.
    Timeseries,
}

impl ProcessingWorkflow {
    /// The string stored in the `jobs.workflow` column.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Sbas       => "sbas",
            Self::PsInsar    => "ps_insar",
            Self::Timeseries => "timeseries",
        }
    }
}

impl Default for ProcessingWorkflow {
    fn default() -> Self {
        Self::Sbas
    }
}

// =============================================================================
// JobRequest
// =============================================================================

/// The body of a job submission request from an interface client.
///
/// All optional fields are filled with server-side defaults when `None`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobRequest {
    // --- Optional processing parameters (filled with defaults if absent) ---
    pub sensor:        Option<Sensor>,
    pub workflow:      Option<ProcessingWorkflow>,
    /// Explicit relative orbit path number. If `None`, the server selects the
    /// best path automatically from the ASF search results.
    pub path:          Option<u32>,
    pub range_looks:   Option<u32>,
    pub azimuth_looks: Option<u32>,
    pub connections:   Option<u8>,
    /// Optional prepaid code UUID for tier upgrade.
    pub prepaid_code:  Option<uuid::Uuid>,

    // --- Required: spatial and temporal bounds ---
    pub north: f64,
    pub south: f64,
    pub east:  f64,
    pub west:  f64,
    pub start: NaiveDate,
    pub end:   NaiveDate,
}

// =============================================================================
// Errors
// =============================================================================

#[derive(Error, Debug)]
pub enum CoordinatesError {
    #[error("latitude must be in [-90, 90]; got {0}")]
    OutOfBoundsLat(f64),
    #[error("longitude must be in [-180, 180]; got {0}")]
    OutOfBoundsLon(f64),
}

#[derive(Error, Debug)]
pub enum BoundingBoxError {
    #[error("invalid coordinates: {0}")]
    InvalidCoordinates(#[from] CoordinatesError),
    #[error("north ({north}) must be greater than south ({south})")]
    InvalidLatSpan { north: f64, south: f64 },
    #[error("east ({east}) must be greater than west ({west})")]
    InvalidLonSpan { east: f64, west: f64 },
}

#[derive(Error, Debug)]
pub enum TimeRangeError {
    #[error("start and end dates must be today or in the past")]
    FutureDate,
    #[error("start date must be before or equal to end date")]
    InvertedBounds,
}

#[derive(Error, Debug)]
pub enum FlightPathError {
    #[error("no images found for the given spatial and temporal bounds")]
    InsufficientMaterial,
    #[error(
        "the specified path has insufficient coverage for the given bounds; \
         leave path unset for automatic selection"
    )]
    InvalidPath,
}
