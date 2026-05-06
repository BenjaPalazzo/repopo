/// sisar-download
///
/// Reads /job/burst_list.json and /job/specification.toml, checks the
/// /archive directory for each burst, downloads any missing ones from
/// the ASF burst extractor, and then:
///   1. Runs local2safe.py to stitch bursts → SAFE directories in /job/data
///   2. Zips each SAFE to maximum compression
///   3. Downloads the Copernicus DEM using dem_stitcher
///   4. Downloads precise orbits using sentineleof
///
/// Credentials are read from EARTHDATA_USER / EARTHDATA_PASS env vars
/// or from ~/.netrc (machine urs.earthdata.nasa.gov / earthdata.nasa.gov).

mod types;
use types::{BurstList, Spec};

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use anyhow::{Context, Result, anyhow, bail};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Run a subprocess, inherit stdio, and return an error if it exits non-zero.
fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let status = cmd
        .status()
        .with_context(|| format!("Failed to launch `{program}`"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "`{}` exited with status {}",
            program,
            status.code().unwrap_or(-1)
        ))
    }
}

/// Return the path where a burst file lives in the archive.
/// Layout: {archive_root}/{slc_granule}/{subswath}/{pol}/{index}.{ext}
fn archive_path(archive_root: &Path, slc: &str, sw: &str, pol: &str, idx: &str, ext: &str) -> PathBuf {
    archive_root
        .join(slc)
        .join(sw)
        .join(pol)
        .join(format!("{idx}.{ext}"))
}

// ── ASF download ──────────────────────────────────────────────────────────────

/// Download a single file from ASF using the Earthdata credentials.
/// Delegates to the `earthdata` Rust library compiled into this binary.
async fn download_burst(
    url: &str,
    dest: &Path,
) -> Result<()> {
    earthdata::download(url, dest).await
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run_download().await {
        eprintln!("[sisar-download] FATAL: {e:?}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_download() -> Result<()> {
    let job_dir = PathBuf::from("/job");
    let archive_root = PathBuf::from("/archive");

    // ── 1. Parse inputs ───────────────────────────────────────────────────────
    let burst_list_path = job_dir.join("burst_list.json");
    let spec_path = job_dir.join("specification.toml");

    eprintln!("[sisar-download] Reading burst_list.json …");
    let burst_list: BurstList = {
        let raw = fs::read_to_string(&burst_list_path)
            .context("Cannot read /job/burst_list.json")?;
        serde_json::from_str(&raw).context("Cannot parse burst_list.json")?
    };

    eprintln!("[sisar-download] Reading specification.toml …");
    let spec: Spec = {
        let raw = fs::read_to_string(&spec_path)
            .context("Cannot read /job/specification.toml")?;
        toml::from_str(&raw).context("Cannot parse specification.toml")?
    };

    // ── 2. Check archive / download missing bursts ────────────────────────────
    eprintln!("[sisar-download] Checking archive for required bursts …");

    // Collect all (url, dest) pairs that need downloading.
    // burst_list structure: { slc: { subswath: { pol: { idx: { DATA, METADATA } } } } }
    let mut missing_tiff: Vec<(String, PathBuf)> = Vec::new();
    let mut missing_xml:  Vec<(String, PathBuf)> = Vec::new();

    for (slc, swaths) in &burst_list {
        let slc_upper = slc.to_uppercase();
        for (sw, pols) in swaths {
            let sw_upper = sw.to_uppercase();
            for (pol, bursts) in pols {
                let pol_upper = pol.to_uppercase();
                for (idx, entry) in bursts {
                    let tiff_dest = archive_path(
                        &archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "tiff",
                    );
                    let xml_dest = archive_path(
                        &archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "xml",
                    );

                    if !tiff_dest.exists() {
                        missing_tiff.push((entry.data.clone(), tiff_dest));
                    }
                    if !xml_dest.exists() {
                        missing_xml.push((entry.metadata.clone(), xml_dest));
                    }
                }
            }
        }
    }

    let total_missing = missing_tiff.len() + missing_xml.len();
    if total_missing == 0 {
        eprintln!("[sisar-download] All bursts present in archive.");
    } else {
        eprintln!(
            "[sisar-download] Downloading {} missing file(s) from ASF …",
            total_missing
        );
        // Download TIFFs and XMLs sequentially to respect Earthdata rate limits.
        // Parallelise within reasonable bounds if needed in the future.
        for (url, dest) in missing_tiff.iter().chain(missing_xml.iter()) {
            eprintln!("[sisar-download]   GET {url}");
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Cannot create dir {}", parent.display()))?;
            }
            download_burst(url, dest)
                .await
                .with_context(|| format!("Failed to download {url}"))?;
        }
        eprintln!("[sisar-download] All downloads complete.");
    }

    // ── 3. Verify the burst_list.json DATA/METADATA paths point into /archive ─
    // The paths written by the API server already point to /archive/…
    // local2safe.py will read them from the JSON directly, so they are correct
    // as long as the archive files are present (which we just ensured above).

    // ── 4. Run local2safe.py → produce SAFE(s) in /job/data ─────────────────
    eprintln!("[sisar-download] Running local2safe …");
    let data_dir = job_dir.join("data");
    fs::create_dir_all(&data_dir).context("Cannot create /job/data")?;

    run(
        "python3",
        &[
            "/usr/local/bin/local2safe",
            burst_list_path.to_str().unwrap(),
            "--all_anns",
            "--work_dir",
            data_dir.to_str().unwrap(),
        ],
        Some(&job_dir),
    )
    .context("local2safe failed")?;

    // ── 5. Zip every .SAFE directory in /job/data at maximum compression ─────
    eprintln!("[sisar-download] Zipping SAFE directories …");
    zip_safes(&data_dir)?;

    // ── 6. Download DEM ───────────────────────────────────────────────────────
    eprintln!("[sisar-download] Downloading DEM …");
    download_dem(&job_dir, &spec)?;

    // ── 7. Download precise orbits ────────────────────────────────────────────
    eprintln!("[sisar-download] Downloading precise orbits …");
    download_orbits(&job_dir)?;

    eprintln!("[sisar-download] Download stage complete.");
    Ok(())
}

// ── zip SAFEs ─────────────────────────────────────────────────────────────────

fn zip_safes(data_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(data_dir).context("Cannot read /job/data")? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow!("Invalid SAFE dir name"))?;
            if name.ends_with(".SAFE") {
                let zip_name = name.replace(".SAFE", ".zip");
                let zip_path = data_dir.join(&zip_name);
                eprintln!("[sisar-download]   Zipping {name} → {zip_name}");
                run(
                    "zip",
                    &[
                        "-r",
                        "-9",
                        zip_path.to_str().unwrap(),
                        name,
                    ],
                    Some(data_dir),
                )
                .with_context(|| format!("zip failed for {name}"))?;
                // Remove the unzipped SAFE directory to save space
                fs::remove_dir_all(&path)
                    .with_context(|| format!("Cannot remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

// ── DEM download ──────────────────────────────────────────────────────────────

fn download_dem(job_dir: &Path, spec: &Spec) -> Result<()> {
    let dem_dir = job_dir.join("dem");
    fs::create_dir_all(&dem_dir).context("Cannot create /job/dem")?;

    let b = &spec.dem.bounds;
    run(
        "python3",
        &[
            "/usr/local/bin/download_dem",
            "--bounds",
            &b.west.to_string(),
            &b.south.to_string(),
            &b.east.to_string(),
            &b.north.to_string(),
            "--output",
            dem_dir.to_str().unwrap(),
        ],
        Some(job_dir),
    )
    .context("DEM download failed")?;
    Ok(())
}

// ── orbit download ────────────────────────────────────────────────────────────

fn download_orbits(job_dir: &Path) -> Result<()> {
    let data_dir = job_dir.join("data");
    let orbits_dir = job_dir.join("orbits");
    fs::create_dir_all(&orbits_dir).context("Cannot create /job/orbits")?;

    // sentineleof scans the directory for S1 zips/SAFEs and downloads matching
    // precise orbit files (POE) into the specified output directory.
    run(
        "eof",
        &[
            "--save-dir",
            orbits_dir.to_str().unwrap(),
            "--search-path",
            data_dir.to_str().unwrap(),
        ],
        Some(job_dir),
    )
    .context("sentineleof (eof) failed")?;
    Ok(())
}

// ── earthdata sub-module (thin wrapper around the library) ───────────────────

mod earthdata {
    pub use earthdata_rs::download;
}
