/// sisar-download
///
/// 1. Reads /job/burst_list.json  — used only for archive verification and download.
///    Downloads any missing bursts from the ASF burst extractor.
/// 2. Reads /job/burst_stitch/{YYYY-mm-dd}.json  — one file per acquisition date,
///    written by the API server. Runs local2safe once per file, producing one SAFE
///    per date. All SAFEs are zipped to /job/data/ at maximum compression.
/// 3. Downloads the Copernicus DEM (bounds from /job/specification.toml [dem.bounds]).
/// 4. Downloads precise orbit files via sentineleof, scanning /job/data/.
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

use anyhow::{Context, Result, anyhow};

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

    // ASF sentinel-1 burst extractor base URL.
    // Full URL pattern:
    //   https://sentinel1-burst.asf.alaska.edu/{SLC_GRANULE}/{SUBSWATH}/{POL}/{INDEX}.tiff
    //   https://sentinel1-burst.asf.alaska.edu/{SLC_GRANULE}/{SUBSWATH}/{POL}/{INDEX}.xml
    //
    // The DATA/METADATA fields in burst_list.json are *archive destination paths*,
    // not download URLs. We derive the ASF URLs from the burst list keys instead.
    const ASF_BURST_BASE: &str = "https://sentinel1-burst.asf.alaska.edu";

    // Collect all (asf_url, archive_dest) pairs that need downloading.
    // burst_list structure: { slc: { subswath: { pol: { idx: { DATA, METADATA } } } } }
    let mut missing_tiff: Vec<(String, PathBuf)> = Vec::new();
    let mut missing_xml:  Vec<(String, PathBuf)> = Vec::new();

    for (slc, swaths) in &burst_list {
        let slc_upper = slc.to_uppercase();
        for (sw, pols) in swaths {
            let sw_upper = sw.to_uppercase();
            for (pol, bursts) in pols {
                let pol_upper = pol.to_uppercase();
                for (idx, _entry) in bursts {
                    // Archive destination paths (where the file will live locally)
                    let tiff_dest = archive_path(
                        &archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "tiff",
                    );
                    let xml_dest = archive_path(
                        &archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "xml",
                    );

                    // ASF download URLs (built from burst list keys)
                    let burst_base = format!(
                        "{ASF_BURST_BASE}/{slc_upper}/{sw_upper}/{pol_upper}/{idx}"
                    );

                    if !tiff_dest.exists() {
                        missing_tiff.push((format!("{burst_base}.tiff"), tiff_dest));
                    }
                    if !xml_dest.exists() {
                        missing_xml.push((format!("{burst_base}.xml"), xml_dest));
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

    // ── 4. Run local2safe once per burst_stitch tree → SAFEs in /job/data ────
    //
    // /job/burst_stitch/ contains one JSON file per acquisition date, named
    // {YYYY-mm-dd}.json, written by the API server. Each file is an independent
    // SLC tree in the same format as burst_list.json. local2safe is called once
    // per file; all resulting SAFEs land in /job/data/ and are zipped in place.
    let burst_stitch_dir = job_dir.join("burst_stitch");
    let data_dir = job_dir.join("data");
    fs::create_dir_all(&data_dir).context("Cannot create /job/data")?;

    // Collect and sort the stitch trees so execution order is deterministic
    // (alphabetical on YYYY-mm-dd is chronological).
    let mut stitch_files: Vec<PathBuf> = fs::read_dir(&burst_stitch_dir)
        .with_context(|| format!("Cannot read {}", burst_stitch_dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    stitch_files.sort();

    if stitch_files.is_empty() {
        return Err(anyhow!("No .json files found in {}", burst_stitch_dir.display()));
    }

    eprintln!(
        "[sisar-download] Running local2safe for {} stitch tree(s) …",
        stitch_files.len()
    );

    for stitch_path in &stitch_files {
        let date_label = stitch_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        eprintln!("[sisar-download]   Stitching {date_label} …");
        run(
            "python3",
            &[
                "/usr/local/bin/local2safe",
                stitch_path.to_str().unwrap(),
                "--all_anns",
                "--work_dir",
                data_dir.to_str().unwrap(),
            ],
            Some(&job_dir),
        )
        .with_context(|| format!("local2safe failed for {date_label}"))?;
    }

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
                // Run zip from data_dir, naming the .SAFE directory explicitly.
                // This produces the canonical ESA layout where the .SAFE folder
                // is the single top-level entry inside the archive, which is what
                // ISCE2's stackSentinel.py / Stack.py expects when it constructs
                // paths like "{SAFE_NAME}.SAFE/preview/map-overlay.kml".
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
