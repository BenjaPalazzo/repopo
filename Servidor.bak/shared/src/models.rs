use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

// =============================================================================
// User tier
// =============================================================================

/// The account-level tier assigned to a user. Determines default submission
/// limits (AOI, time range, monthly quota) enforced by the API server.
///
/// The scheduler uses this indirectly via `Job::effective_tier`, which may
/// differ from the user's account tier when a prepaid code was redeemed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserTier {
    /// Restricted access, intended for evaluation. Smallest limits.
    Demo,
    /// Standard unpaid access.
    Free,
    /// Paid tier with the highest (or unlimited) limits.
    Pro,
}

impl UserTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Free => "free",
            Self::Pro => "pro",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "demo" => Some(Self::Demo),
            "free" => Some(Self::Free),
            "pro" => Some(Self::Pro),
            _ => None,
        }
    }

    /// Numeric priority used by the scheduler when ordering jobs for dispatch.
    /// Higher value = claimed first.
    pub fn dispatch_priority(&self) -> i32 {
        match self {
            Self::Pro => 2,
            Self::Free => 1,
            Self::Demo => 0,
        }
    }
}

// =============================================================================
// ISCE2 stages
// =============================================================================

/// Each variant maps to a snake_case argument passed to the ISCE2 container
/// entrypoint. The order reflects the topsStack execution sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsceStage {
    UnpackTopoReference,
    UnpackSecondarySlc,
    AverageBaseline,
    ExtractBurstOverlaps,
    OverlapGeoToRadar,
    OverlapResample,
    PairsMisreg,
    TimeseriesMisreg,
    FullBurstGeoToRadar,
    FullBurstResample,
    ExtractStackValidRegion,
    MergeReferenceSecondarySlc,
    GenerateBurstIgram,
    MergeBurstIgram,
    FilterCoherence,
    PhaseUnwrap,
}

impl IsceStage {
    pub fn first() -> Self {
        Self::UnpackTopoReference
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnpackTopoReference => "unpack_topo_reference",
            Self::UnpackSecondarySlc => "unpack_secondary_slc",
            Self::AverageBaseline => "average_baseline",
            Self::ExtractBurstOverlaps => "extract_burst_overlaps",
            Self::OverlapGeoToRadar => "overlap_geo_to_radar",
            Self::OverlapResample => "overlap_resample",
            Self::PairsMisreg => "pairs_misreg",
            Self::TimeseriesMisreg => "timeseries_misreg",
            Self::FullBurstGeoToRadar => "full_burst_geo_to_radar",
            Self::FullBurstResample => "full_burst_resample",
            Self::ExtractStackValidRegion => "extract_stack_valid_region",
            Self::MergeReferenceSecondarySlc => "merge_reference_secondary_slc",
            Self::GenerateBurstIgram => "generate_burst_igram",
            Self::MergeBurstIgram => "merge_burst_igram",
            Self::FilterCoherence => "filter_coherence",
            Self::PhaseUnwrap => "phase_unwrap",
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Self::UnpackTopoReference => Some(Self::UnpackSecondarySlc),
            Self::UnpackSecondarySlc => Some(Self::AverageBaseline),
            Self::AverageBaseline => Some(Self::ExtractBurstOverlaps),
            Self::ExtractBurstOverlaps => Some(Self::OverlapGeoToRadar),
            Self::OverlapGeoToRadar => Some(Self::OverlapResample),
            Self::OverlapResample => Some(Self::PairsMisreg),
            Self::PairsMisreg => Some(Self::TimeseriesMisreg),
            Self::TimeseriesMisreg => Some(Self::FullBurstGeoToRadar),
            Self::FullBurstGeoToRadar => Some(Self::FullBurstResample),
            Self::FullBurstResample => Some(Self::ExtractStackValidRegion),
            Self::ExtractStackValidRegion => Some(Self::MergeReferenceSecondarySlc),
            Self::MergeReferenceSecondarySlc => Some(Self::GenerateBurstIgram),
            Self::GenerateBurstIgram => Some(Self::MergeBurstIgram),
            Self::MergeBurstIgram => Some(Self::FilterCoherence),
            Self::FilterCoherence => Some(Self::PhaseUnwrap),
            Self::PhaseUnwrap => None,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "unpack_topo_reference" => Some(Self::UnpackTopoReference),
            "unpack_secondary_slc" => Some(Self::UnpackSecondarySlc),
            "average_baseline" => Some(Self::AverageBaseline),
            "extract_burst_overlaps" => Some(Self::ExtractBurstOverlaps),
            "overlap_geo_to_radar" => Some(Self::OverlapGeoToRadar),
            "overlap_resample" => Some(Self::OverlapResample),
            "pairs_misreg" => Some(Self::PairsMisreg),
            "timeseries_misreg" => Some(Self::TimeseriesMisreg),
            "full_burst_geo_to_radar" => Some(Self::FullBurstGeoToRadar),
            "full_burst_resample" => Some(Self::FullBurstResample),
            "extract_stack_valid_region" => Some(Self::ExtractStackValidRegion),
            "merge_reference_secondary_slc" => Some(Self::MergeReferenceSecondarySlc),
            "generate_burst_igram" => Some(Self::GenerateBurstIgram),
            "merge_burst_igram" => Some(Self::MergeBurstIgram),
            "filter_coherence" => Some(Self::FilterCoherence),
            "phase_unwrap" => Some(Self::PhaseUnwrap),
            _ => None,
        }
    }

    pub fn resource_tier(&self) -> ResourceTier {
        match self {
            Self::UnpackTopoReference
            | Self::UnpackSecondarySlc
            | Self::ExtractBurstOverlaps
            | Self::ExtractStackValidRegion
            | Self::MergeReferenceSecondarySlc
            | Self::MergeBurstIgram => ResourceTier::Light,

            Self::AverageBaseline
            | Self::OverlapGeoToRadar
            | Self::OverlapResample
            | Self::PairsMisreg
            | Self::TimeseriesMisreg
            | Self::FullBurstGeoToRadar
            | Self::FullBurstResample
            | Self::GenerateBurstIgram
            | Self::FilterCoherence => ResourceTier::Medium,

            Self::PhaseUnwrap => ResourceTier::Heavy,
        }
    }
}

// =============================================================================
// MintPy stages  (placeholder)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MintpyStage {
    LoadData,
    ModifyNetwork,
    ReferencePoint,
    QuickOverview,
    CorrectUnwrapError,
    InvertNetwork,
    CorrectLod,
    CorrectSet,
    CorrectIonosphere,
    CorrectTroposphere,
    Deramp,
    CorrectTopography,
    ResidualRms,
    ReferenceDate,
    Velocity,
    Geocode,
    GoogleEarth,
    HdfEos5,
}

impl MintpyStage {
    pub fn first() -> Self {
        Self::LoadData
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LoadData => "load_data",
            Self::ModifyNetwork => "modify_network",
            Self::ReferencePoint => "reference_point",
            Self::QuickOverview => "quick_overview",
            Self::CorrectUnwrapError => "correct_unwrap_error",
            Self::InvertNetwork => "invert_network",
            Self::CorrectLod => "correct_LOD",
            Self::CorrectSet => "correct_SET",
            Self::CorrectIonosphere => "correct_ionosphere",
            Self::CorrectTroposphere => "correct_troposphere",
            Self::Deramp => "deramp",
            Self::CorrectTopography => "correct_topography",
            Self::ResidualRms => "residual_RMS",
            Self::ReferenceDate => "reference_date",
            Self::Velocity => "velocity",
            Self::Geocode => "geocode",
            Self::GoogleEarth => "google_earth",
            Self::HdfEos5 => "hdfeos5",
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Self::LoadData => Some(Self::ModifyNetwork),
            Self::ModifyNetwork => Some(Self::ReferencePoint),
            Self::ReferencePoint => Some(Self::QuickOverview),
            Self::QuickOverview => Some(Self::CorrectUnwrapError),
            Self::CorrectUnwrapError => Some(Self::InvertNetwork),
            Self::InvertNetwork => Some(Self::CorrectLod),
            Self::CorrectLod => Some(Self::CorrectSet),
            Self::CorrectSet => Some(Self::CorrectIonosphere),
            Self::CorrectIonosphere => Some(Self::CorrectTroposphere),
            Self::CorrectTroposphere => Some(Self::Deramp),
            Self::Deramp => Some(Self::CorrectTopography),
            Self::CorrectTopography => Some(Self::ResidualRms),
            Self::ResidualRms => Some(Self::ReferenceDate),
            Self::ReferenceDate => Some(Self::Velocity),
            Self::Velocity => Some(Self::Geocode),
            Self::Geocode => Some(Self::GoogleEarth),
            Self::GoogleEarth => Some(Self::HdfEos5),
            Self::HdfEos5 => None,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "load_data" => Some(Self::LoadData),
            "modify_network" => Some(Self::ModifyNetwork),
            "reference_point" => Some(Self::ReferencePoint),
            "quick_overview" => Some(Self::QuickOverview),
            "correct_unwrap_error" => Some(Self::CorrectUnwrapError),
            "invert_network" => Some(Self::InvertNetwork),
            "correct_LOD" => Some(Self::CorrectLod),
            "correct_SET" => Some(Self::CorrectSet),
            "correct_ionosphere" => Some(Self::CorrectIonosphere),
            "correct_troposphere" => Some(Self::CorrectTroposphere),
            "deramp" => Some(Self::Deramp),
            "correct_topography" => Some(Self::CorrectTopography),
            "residual_RMS" => Some(Self::ResidualRms),
            "reference_date" => Some(Self::ReferenceDate),
            "velocity" => Some(Self::Velocity),
            "geocode" => Some(Self::Geocode),
            "google_earth" => Some(Self::GoogleEarth),
            "hdfeos5" => Some(Self::HdfEos5),
            _ => None,
        }
    }

    pub fn resource_tier(&self) -> ResourceTier {
        match self {
            Self::LoadData
            | Self::ModifyNetwork
            | Self::ReferencePoint
            | Self::QuickOverview
            | Self::ReferenceDate
            | Self::GoogleEarth
            | Self::HdfEos5 => ResourceTier::Light,

            Self::CorrectUnwrapError
            | Self::InvertNetwork
            | Self::CorrectLod
            | Self::CorrectSet
            | Self::CorrectIonosphere
            | Self::CorrectTroposphere
            | Self::Deramp
            | Self::CorrectTopography
            | Self::ResidualRms
            | Self::Geocode => ResourceTier::Medium,

            Self::Velocity => ResourceTier::Heavy,
        }
    }
}

// =============================================================================
// MiaplPy stages  (placeholder)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiaplpyStage {
    // TODO: define variants
}

impl MiaplpyStage {
    pub fn as_str(&self) -> &'static str {
        match *self {}
    }
    pub fn next(&self) -> Option<Self> {
        match *self {}
    }
    pub fn from_str(_s: &str) -> Option<Self> {
        None
    }
    pub fn resource_tier(&self) -> ResourceTier {
        match *self {}
    }
}

// =============================================================================
// Resource tier
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceTier {
    Light,
    Medium,
    Heavy,
}

// =============================================================================
// Job error
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobError {
    /// Container exited with a non-zero status code.
    #[error("container exited with non-zero status: {exit_code}")]
    ContainerFailed { exit_code: i64 },

    /// Container could not be started (image missing, Docker daemon error, etc).
    #[error("container failed to start: {reason}")]
    ContainerStartFailed { reason: String },

    /// Container exceeded its allowed wall-clock runtime.
    #[error("container timed out")]
    Timeout,

    /// Scheduler-internal error unrelated to the container itself.
    #[error("internal scheduler error: {message}")]
    Internal { message: String },
}

impl JobError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ContainerFailed { .. } => "container_failed",
            Self::ContainerStartFailed { .. } => "container_start_failed",
            Self::Timeout => "timeout",
            Self::Internal { .. } => "internal",
        }
    }

    /// Encodes variant-specific detail into the `job_error_message` column.
    pub fn to_db_string(&self) -> Option<String> {
        match self {
            Self::ContainerFailed { exit_code } => Some(exit_code.to_string()),
            Self::ContainerStartFailed { reason } => Some(reason.clone()),
            Self::Timeout => None,
            Self::Internal { message } => Some(message.clone()),
        }
    }

    pub fn from_db(kind: &str, message: &str) -> Option<Self> {
        match kind {
            "container_failed" => {
                let exit_code = message.parse().unwrap_or(-1);
                Some(Self::ContainerFailed { exit_code })
            }
            "container_start_failed" => Some(Self::ContainerStartFailed {
                reason: message.to_string(),
            }),
            "timeout" => Some(Self::Timeout),
            "internal" => Some(Self::Internal {
                message: message.to_string(),
            }),
            _ => None,
        }
    }
}

// =============================================================================
// Job state
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    /// API server has registered the job; scheduler has not yet picked it up.
    Queued,
    /// API server is writing spec files and scaffolding directories.
    Initializing,
    /// Download container is running.
    Downloading,
    /// ISCE2 processing container is running the given stage.
    IsceProcessing { stage: IsceStage },
    /// MintPy container is running the given stage.
    MintpyProcessing { stage: MintpyStage },
    /// MiaplPy container is running the given stage.
    MiaplpyProcessing { stage: MiaplpyStage },
    /// Results container is generating output products (e.g. velocity.png).
    ResultsGenerating,
    /// All steps completed successfully.
    Completed,
    /// A critical error halted processing.
    Failed { error: JobError },
    /// Cancelled by user or API server.
    Cancelled,
}

impl JobState {
    pub fn is_advanceable(&self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Downloading
                | Self::IsceProcessing { .. }
                | Self::MintpyProcessing { .. }
                | Self::MiaplpyProcessing { .. }
                | Self::ResultsGenerating
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed { .. } | Self::Cancelled
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Initializing => "initializing",
            Self::Downloading => "downloading",
            Self::IsceProcessing { .. } => "isce_processing",
            Self::MintpyProcessing { .. } => "mintpy_processing",
            Self::MiaplpyProcessing { .. } => "miaplpy_processing",
            Self::ResultsGenerating => "results_generating",
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn as_stage_str(&self) -> Option<String> {
        match self {
            Self::IsceProcessing { stage } => Some(stage.as_str().to_string()),
            Self::MintpyProcessing { stage } => Some(stage.as_str().to_string()),
            Self::MiaplpyProcessing { stage } => Some(stage.as_str().to_string()),
            _ => None,
        }
    }
}

// =============================================================================
// Job
// =============================================================================

/// A fully typed job record, converted from a raw `JobRow`.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub user_id: Uuid,
    pub workflow: String,
    pub state: JobState,
    /// The tier whose resource limits apply to this specific job. May differ
    /// from the owning user's account tier when a prepaid code was redeemed.
    pub effective_tier: UserTier,
    pub work_dir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// JobRow — raw sqlx mapping
// =============================================================================

/// Direct mapping of the `jobs` table row. Only used at the DB boundary.
#[derive(Debug, sqlx::FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub workflow: String,
    pub job_state: String,
    pub job_stage: Option<String>,
    pub job_error_kind: Option<String>,
    pub job_error_message: Option<String>,
    pub effective_tier: String,
    pub work_dir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// JobRow -> Job conversion
// =============================================================================

#[derive(Debug, Error)]
pub enum JobRowError {
    #[error("unknown job_state value: {0:?}")]
    UnknownState(String),

    #[error("job_state {state:?} requires a job_stage, but none was present")]
    MissingStage { state: String },

    #[error("unknown job_stage value {stage:?} for state {state:?}")]
    UnknownStage { state: String, stage: String },

    #[error("job_state 'failed' is missing job_error_kind")]
    MissingErrorKind,

    #[error("unknown effective_tier value: {0:?}")]
    UnknownTier(String),
}

impl TryFrom<JobRow> for Job {
    type Error = JobRowError;

    fn try_from(row: JobRow) -> Result<Self, Self::Error> {
        let state = parse_job_state(
            &row.job_state,
            row.job_stage.as_deref(),
            row.job_error_kind.as_deref(),
            row.job_error_message.as_deref(),
        )?;

        let effective_tier = UserTier::from_str(&row.effective_tier)
            .ok_or_else(|| JobRowError::UnknownTier(row.effective_tier.clone()))?;

        Ok(Job {
            id: row.id,
            user_id: row.user_id,
            workflow: row.workflow,
            state,
            effective_tier,
            work_dir: row.work_dir,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn parse_job_state(
    state: &str,
    stage: Option<&str>,
    error_kind: Option<&str>,
    error_message: Option<&str>,
) -> Result<JobState, JobRowError> {
    match state {
        "queued" => Ok(JobState::Queued),
        "initializing" => Ok(JobState::Initializing),
        "downloading" => Ok(JobState::Downloading),
        "results_generating" => Ok(JobState::ResultsGenerating),
        "completed" => Ok(JobState::Completed),
        "cancelled" => Ok(JobState::Cancelled),

        "isce_processing" => {
            let s = stage.ok_or_else(|| JobRowError::MissingStage {
                state: state.to_string(),
            })?;
            let stage = IsceStage::from_str(s).ok_or_else(|| JobRowError::UnknownStage {
                state: state.to_string(),
                stage: s.to_string(),
            })?;
            Ok(JobState::IsceProcessing { stage })
        }

        "mintpy_processing" => {
            let s = stage.ok_or_else(|| JobRowError::MissingStage {
                state: state.to_string(),
            })?;
            let stage = MintpyStage::from_str(s).ok_or_else(|| JobRowError::UnknownStage {
                state: state.to_string(),
                stage: s.to_string(),
            })?;
            Ok(JobState::MintpyProcessing { stage })
        }

        "miaplpy_processing" => {
            let s = stage.ok_or_else(|| JobRowError::MissingStage {
                state: state.to_string(),
            })?;
            let stage = MiaplpyStage::from_str(s).ok_or_else(|| JobRowError::UnknownStage {
                state: state.to_string(),
                stage: s.to_string(),
            })?;
            Ok(JobState::MiaplpyProcessing { stage })
        }

        "failed" => {
            let kind = error_kind.ok_or(JobRowError::MissingErrorKind)?;
            let message = error_message.unwrap_or("");
            let error = JobError::from_db(kind, message).unwrap_or_else(|| JobError::Internal {
                message: format!("unrecognised error kind in DB: {kind}"),
            });
            Ok(JobState::Failed { error })
        }

        other => Err(JobRowError::UnknownState(other.to_string())),
    }
}
