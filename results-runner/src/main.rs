//! sisar-results — results container entrypoint (Rust replacement for entrypoint.sh)
//!
//! Subcommands:
//!   velocity   --output <path>
//!   timeseries --lat <f64> --lon <f64> --output <path>
//!
//! Stub subcommands (exit 1, not yet implemented):
//!   ai_summary
//!   dem
//!   3d_model
//!
//! All heavy lifting is delegated to MintPy CLI tools (`view.py`, `tsview.py`)
//! which are expected to be on PATH inside the container.
//!
//! For `timeseries`, `tsview.py` generates a PDF; this binary then converts it
//! to PNG using `pdftocairo` (from poppler-utils) and removes the intermediate
//! PDF.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser)]
#[command(
    name = "sisar-results",
    about = "SISAR results container — generates output artifacts from MintPy data"
)]
struct Cli {
    #[command(subcommand)]
    command: SubCommand,
}

#[derive(Subcommand)]
enum SubCommand {
    /// Generate the velocity map PNG from geo_velocity.h5
    Velocity {
        /// Absolute path inside the container where velocity.png will be written.
        #[arg(long)]
        output: PathBuf,
    },

    /// Generate a time-series PNG for a specific lat/lon point
    Timeseries {
        #[arg(long)]
        lat: f64,

        #[arg(long)]
        lon: f64,

        /// Absolute path inside the container where the final PNG will be written.
        #[arg(long)]
        output: PathBuf,
    },

    /// (Not yet implemented)
    AiSummary {},

    /// (Not yet implemented)
    Dem {},

    /// (Not yet implemented — note: clap doesn't allow hyphens in variant names,
    /// so the subcommand is registered as "3d-model" via the rename attribute)
    #[command(name = "3d_model")]
    Model3d {},
}

// =============================================================================
// Main
// =============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    // JOB_DIR defaults to /job (the container workdir).
    let job_dir = std::env::var("JOB_DIR").unwrap_or_else(|_| "/job".to_string());
    let job_dir = Path::new(&job_dir);

    match cli.command {
        SubCommand::Velocity { output } => {
            run_velocity(job_dir, &output)
        }

        SubCommand::Timeseries { lat, lon, output } => {
            run_timeseries(job_dir, lat, lon, &output)
        }

        SubCommand::AiSummary {} | SubCommand::Dem {} | SubCommand::Model3d {} => {
            bail!("this result type is not yet implemented")
        }
    }
}

// =============================================================================
// velocity
// =============================================================================

fn run_velocity(job_dir: &Path, output: &Path) -> Result<()> {
    let series_dir  = job_dir.join("series").join("geo");
    let velocity_h5 = series_dir.join("geo_velocity.h5");
    let mask_h5     = series_dir.join("geo_maskTempCoh.h5");
    let dem_file    = job_dir.join("dem").join("full_res.dem.wgs84");

    require_file(&velocity_h5)?;
    require_file(&mask_h5)?;
    require_file(&dem_file)?;

    ensure_parent_dir(output)?;

    eprintln!(
        "[sisar/results] Generating velocity map → {}",
        output.display()
    );

    run_command(
        "view.py",
        &[
            velocity_h5.to_str().unwrap(),
            "velocity",
            "-m", mask_h5.to_str().unwrap(),
            "-d", dem_file.to_str().unwrap(),
            "-v", "-10", "10",
            "--notitle",
            "--nodisplay",
            "-o", output.to_str().unwrap(),
        ],
    )
    .context("view.py failed")?;

    eprintln!("[sisar/results] velocity.png written successfully");
    Ok(())
}

// =============================================================================
// timeseries
// =============================================================================

fn run_timeseries(job_dir: &Path, lat: f64, lon: f64, output: &Path) -> Result<()> {
    let series_dir    = job_dir.join("series").join("geo");
    let timeseries_h5 = series_dir.join("geo_timeseries_ERA5_ramp_demErr.h5");

    require_file(&timeseries_h5)?;
    ensure_parent_dir(output)?;

    // tsview.py writes a PDF; we use a temp path alongside the final output.
    let results_dir = output.parent().unwrap_or(Path::new("."));
    let pdf_path = results_dir.join(format!("series_{lat}_{lon}.pdf"));

    eprintln!(
        "[sisar/results] Generating time-series for ({lat}, {lon}) → {} (PDF intermediate)",
        pdf_path.display()
    );

    run_command(
        "tsview.py",
        &[
            timeseries_h5.to_str().unwrap(),
            "--zf",
            "-u", "cm",
            "--lalo", &lat.to_string(), &lon.to_string(),
            "--nodisplay",
            "-o", pdf_path.to_str().unwrap(),
        ],
    )
    .context("tsview.py failed")?;

    let pdf_path = results_dir.join(format!("series_{lat}_{lon}_ts.pdf"));

    require_file(&pdf_path)?;

    eprintln!(
        "[sisar/results] Converting PDF → PNG: {}",
        output.display()
    );

    // pdftocairo -png -r 150 -singlefile <in.pdf> <out_stem>
    // pdftocairo appends ".png" to the stem, so we strip the extension.
    let png_stem = results_dir.join(format!("series_{lat}_{lon}_ts"));

    run_command(
        "pdftocairo",
        &[
            "-png",
            "-r", "150",
            "-singlefile",
            pdf_path.to_str().unwrap(),
            png_stem.to_str().unwrap(),
        ],
    )
    .context("pdftocairo failed")?;

    // Remove the intermediate PDF.
    if pdf_path.exists() {
        std::fs::remove_file(&pdf_path)
            .with_context(|| format!("removing intermediate PDF {}", pdf_path.display()))?;
    }

    // Verify the PNG was actually produced.
    let final_png = results_dir.join(format!("series_{lat}_{lon}_ts.png"));
    require_file(&final_png)?;

    eprintln!("[sisar/results] timeseries PNG written successfully");
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

/// Ensures a required input file exists, returning a clear error if not.
fn require_file(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("required file not found: {}", path.display());
    }
    Ok(())
}

/// Creates the parent directory of a path if it does not exist.
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }
    Ok(())
}

/// Runs an external command, inheriting stdout/stderr, and returns an error
/// if the process exits with a non-zero status.
fn run_command(program: &str, args: &[&str]) -> Result<()> {
    eprintln!("[sisar/results] + {program} {}", args.join(" "));

    let status: ExitStatus = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch '{program}'"))?;

    if !status.success() {
        bail!(
            "'{program}' exited with status {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}
