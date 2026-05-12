//! mintpy-runner — MintPy stage dispatcher for the SISAR pipeline.
//!
//! Usage: mintpy-runner <stage_name>
//!
//! The runner receives a single snake_case MintPy stage name, executes the
//! corresponding smallbaselineApp.py step, and exits 0 on success or non-zero
//! on failure. All diagnostic output goes to stderr.
//!
//! Reads /job/specification.toml for job parameters (bounds, tropo method, etc.)
//! Results are written to /jobs/results/<job_id> as configured by the scheduler.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

// ---------------------------------------------------------------------------
// IW*.xml parser — produces data.rsc without requiring ISCE to be installed.
//
// The ISCE2 XML format stores every field as:
//   <property name="fieldname"><value>...</value></property>
// All property name attributes are lowercased by ISCE. We extract values with
// a simple line-by-line state machine rather than a full XML parser to avoid
// extra dependencies.
// ---------------------------------------------------------------------------

/// Extract the text content of every <value>…</value> element that immediately
/// follows a <property name="NAME"> line, returning a flat Vec of (name, value)
/// pairs in document order. Names are lowercased; values are trimmed strings.
fn parse_isce_xml_properties(xml: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current_prop: Option<String> = None;

    for line in xml.lines() {
        let trimmed = line.trim();
        // Match:  <property name="something">
        if trimmed.starts_with("<property name=\"") {
            let inner = &trimmed["<property name=\"".len()..];
            if let Some(end) = inner.find('"') {
                current_prop = Some(inner[..end].to_lowercase());
            }
        // Match:  <value>content</value>
        } else if trimmed.starts_with("<value>") && trimmed.ends_with("</value>") {
            if let Some(name) = current_prop.take() {
                let content = &trimmed["<value>".len()..trimmed.len() - "</value>".len()];
                pairs.push((name, content.trim().to_string()));
            }
        // Any other tag resets the property context so we don't accidentally
        // pick up a <value> that belongs to a nested component, not a property.
        } else if trimmed.starts_with('<') && !trimmed.starts_with("<!--") {
            current_prop = None;
        }
    }
    pairs
}

/// Return the first value matching `name` from a property list.
fn get_prop<'a>(props: &'a [(String, String)], name: &str) -> Option<&'a str> {
    props
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Return *all* values matching `name` (for repeated fields such as burststartutc).
fn get_all_props<'a>(props: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    props
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// Parse a datetime string like "2016-01-17 23:26:16.725734" into seconds-of-day.
fn utc_to_seconds(dt: &str) -> Result<f64> {
    // Format: "YYYY-MM-DD HH:MM:SS.ffffff"
    let time_part = dt
        .split_whitespace()
        .nth(1)
        .with_context(|| format!("No time part in datetime: {dt}"))?;
    let mut parts = time_part.splitn(3, ':');
    let h: f64 = parts.next().context("Missing hours")?.parse()?;
    let m: f64 = parts.next().context("Missing minutes")?.parse()?;
    let s: f64 = parts.next().context("Missing seconds")?.parse()?;
    Ok(h * 3600.0 + m * 60.0 + s)
}

/// Metadata extracted from one IW*.xml file.
struct IwMeta {
    swath_number: u32,
    starting_range: f64,        // burst1 startingrange (m)
    radar_wavelength: f64,      // radarwavelength (m)
    range_pixel_size: f64,      // rangepixelsize — single-look (m)
    azimuth_time_interval: f64, // azimuthtimeinterval (s)
    pass_direction: String,     // ASCENDING / DESCENDING
    polarization: String,
    orbit_number: u32,
    track_number: u32,
    spacecraft_name: String,
    start_utc: String, // burststartutc of burst1
    stop_utc: String,  // burststoputc of last burst
    prf: f64,          // pulserepetitionfrequency (Hz)
}

fn parse_iw_xml(path: &Path) -> Result<IwMeta> {
    let xml =
        std::fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let props = parse_isce_xml_properties(&xml);

    // Helper for required fields (returns first occurrence = burst1 for per-burst fields)
    let req = |name: &str| -> Result<&str> {
        get_prop(&props, name)
            .with_context(|| format!("Missing property '{}' in {}", name, path.display()))
    };

    let starting_range: f64 = req("startingrange")?.parse()?;
    let radar_wavelength: f64 = req("radarwavelength")?.parse()?;
    let range_pixel_size: f64 = req("rangepixelsize")?.parse()?;
    let azimuth_time_interval: f64 = req("azimuthtimeinterval")?.parse()?;
    let prf: f64 = req("pulserepetitionfrequency")?.parse()?;
    let swath_number: u32 = req("swathnumber")?.parse()?;
    let orbit_number: u32 = req("orbitnumber")?.parse()?;
    let track_number: u32 = req("tracknumber")?.parse()?;
    let pass_direction = req("passdirection")?.to_uppercase();
    let polarization = req("polarization")?.to_uppercase();
    let spacecraft_name = req("spacecraftname")?.to_string();

    // All burst start/stop timestamps — first = burst1, last = final burst
    let starts = get_all_props(&props, "burststartutc");
    let stops = get_all_props(&props, "burststoputc");

    let start_utc = starts
        .first()
        .with_context(|| format!("No burststartutc in {}", path.display()))?
        .to_string();
    let stop_utc = stops
        .last()
        .with_context(|| format!("No burststoputc in {}", path.display()))?
        .to_string();

    Ok(IwMeta {
        swath_number,
        starting_range,
        radar_wavelength,
        range_pixel_size,
        azimuth_time_interval,
        pass_direction,
        polarization,
        orbit_number,
        track_number,
        spacecraft_name,
        start_utc,
        stop_utc,
        prf,
    })
}

/// Read WIDTH and LENGTH from an ISCE XML sidecar (e.g. hgt.rdr.xml).
/// Returns (width, length) or None if the file doesn't exist or is missing keys.
fn read_geom_xml_size(xml_path: &Path) -> Option<(u32, u32)> {
    let xml = std::fs::read_to_string(xml_path).ok()?;
    let props = parse_isce_xml_properties(&xml);
    let width: u32 = get_prop(&props, "width")?.parse().ok()?;
    let length: u32 = get_prop(&props, "length")?.parse().ok()?;
    Some((width, length))
}

/// Compute (ALOOKS, RLOOKS) by comparing hgt.rdr.full.xml to hgt.rdr.xml.
/// Falls back to (1, 1) if either sidecar is absent.
fn compute_looks(geom_dir: &Path) -> Result<(u32, u32)> {
    let full_xml = geom_dir.join("hgt.rdr.full.xml");
    let mli_xml = geom_dir.join("hgt.rdr.xml");

    match (read_geom_xml_size(&full_xml), read_geom_xml_size(&mli_xml)) {
        (Some((w_full, l_full)), Some((w_mli, l_mli))) => {
            let rlooks = (w_full / w_mli).max(1);
            let alooks = (l_full / l_mli).max(1);
            eprintln!("[mintpy-runner] Looks from xml: RLOOKS={rlooks} ALOOKS={alooks}");
            Ok((alooks, rlooks))
        }
        _ => {
            eprintln!(
                "[mintpy-runner] WARN: cannot determine looks \
                 (missing {0} or {1}), defaulting to 1×1",
                full_xml.display(),
                mli_xml.display()
            );
            Ok((1, 1))
        }
    }
}

/// Return paths inside `dir` whose filename matches the simple glob `pattern`
/// (only a single `*` wildcard is supported, e.g. "IW*.xml").
fn glob_local(dir: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let (prefix, suffix) = pattern.split_once('*').unwrap_or((pattern, ""));

    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Cannot read directory {}", dir.display()))?;

    let mut results = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix)
            && name.ends_with(suffix)
            && name.len() > prefix.len() + suffix.len()
        {
            results.push(entry.path());
        }
    }
    Ok(results)
}

/// Build and write `{reference_dir}/data.rsc` from all IW*.xml files found
/// in `reference_dir`. Skips writing if an up-to-date file already exists.
///
/// This replicates the metadata extraction logic of
/// `mintpy.utils.isce_utils.extract_tops_metadata` without importing ISCE.
fn build_data_rsc(reference_dir: &Path, geom_dir: &Path) -> Result<()> {
    // Collect and sort IW*.xml files by name (IW1 < IW2 < IW3)
    let mut iw_files = glob_local(reference_dir, "IW*.xml")?;
    if iw_files.is_empty() {
        bail!("No IW*.xml files found in {}", reference_dir.display());
    }
    iw_files.sort();

    let rsc_path = reference_dir.join("data.rsc");

    // Skip writing if rsc is newer than every IW xml (idempotent re-runs)
    if rsc_path.exists() {
        let rsc_mtime = std::fs::metadata(&rsc_path)?.modified()?;
        let all_older = iw_files.iter().all(|f| {
            std::fs::metadata(f)
                .and_then(|m| m.modified())
                .map(|t| t <= rsc_mtime)
                .unwrap_or(false)
        });
        if all_older {
            eprintln!(
                "[mintpy-runner] data.rsc is up to date, skipping: {}",
                rsc_path.display()
            );
            return Ok(());
        }
    }

    eprintln!(
        "[mintpy-runner] Building data.rsc from {} IW xml file(s)…",
        iw_files.len()
    );

    // Parse all IW xmls; sort by swath number
    let mut metas: Vec<IwMeta> = iw_files
        .iter()
        .map(|f| parse_iw_xml(f))
        .collect::<Result<_>>()?;
    metas.sort_by_key(|m| m.swath_number);

    // Reference values from the IW with the lowest swath number (= IW1),
    // matching isce_utils.extract_tops_metadata which uses obj.bursts[0].
    let ref_iw = &metas[0];

    // Sentinel-1 satellite speed (m/s).
    // ISCE2 computes this from the orbit state vector via Hermite interpolation.
    // We use the standard LEO value for Sentinel-1 (~7581 m/s) which matches
    // the example data.rsc to 4 significant figures. If higher precision is
    // needed the orbit state vectors inside the XML can be integrated here.
    const SAT_SPEED: f64 = 7581.695;

    // Single-look azimuth pixel size (m)
    let azimuth_pixel_size_sl = SAT_SPEED * ref_iw.azimuth_time_interval;

    // Beam-swath: concatenated sorted swath numbers, e.g. "123" for all three
    let beam_swath: String = metas.iter().map(|m| m.swath_number.to_string()).collect();

    let orbit_direction = &ref_iw.pass_direction; // "ASCENDING" or "DESCENDING"

    // Platform name: "Sentinel-1" → "sen"
    let platform = if ref_iw.spacecraft_name.to_lowercase().contains("sentinel") {
        "sen".to_string()
    } else {
        ref_iw.spacecraft_name.to_lowercase()
    };

    // Sensing start = burststartutc of burst1 of the lowest-swath IW
    // Sensing stop  = burststoputc  of the last burst of the highest-swath IW
    let start_utc = &ref_iw.start_utc;
    let stop_utc = &metas.last().unwrap().stop_utc;

    // CENTER_LINE_UTC: seconds of day at sensing mid
    let t_start = utc_to_seconds(start_utc)?;
    let t_stop = utc_to_seconds(stop_utc)?;
    let center_line_utc = (t_start + t_stop) / 2.0;

    // WIDTH / LENGTH from geometry sidecar (try hgt first, then lat, then lon)
    let (width, length) = ["hgt.rdr.full.xml", "lat.rdr.full.xml", "lon.rdr.full.xml"]
        .iter()
        .find_map(|name| read_geom_xml_size(&geom_dir.join(name)))
        .with_context(|| {
            format!(
                "Cannot determine WIDTH/LENGTH: no readable geometry xml in {}",
                geom_dir.display()
            )
        })?;

    // Sentinel-1 IW fixed spatial resolutions
    const AZIMUTH_RESOLUTION: f64 = 22.5; // m
    const RANGE_RESOLUTION: f64 = 2.7; // m

    // ALOOKS / RLOOKS from full-res vs multilooked geometry sidecar
    let (alooks, rlooks) = compute_looks(geom_dir)?;

    // Multilooked pixel sizes (matching isce_utils.extract_geometry_metadata)
    let range_pixel_size_ml = ref_iw.range_pixel_size * rlooks as f64;
    let azimuth_pixel_size_ml = azimuth_pixel_size_sl * alooks as f64;

    // NCORRLOOKS: coherence calibration factor
    let rg_fact = RANGE_RESOLUTION / range_pixel_size_ml;
    let az_fact = AZIMUTH_RESOLUTION / azimuth_pixel_size_ml;
    let ncorrlooks = (rlooks * alooks) as f64 / (rg_fact * az_fact);

    // Earth radius and platform altitude.
    // ISCE2 derives these from orbit state vectors via Planet/ellipsoid objects.
    // Those vectors are buried deep in the XML and require orbit integration to
    // reproduce exactly. The constants below are the well-known values for
    // Sentinel-1 and are accurate enough for MintPy's range_distance() call,
    // where they contribute only a small far-range correction (<1%).
    // If sub-percent precision is required, parse the orbit state vector block
    // and integrate it using Hermite interpolation (same as ISCE2 does).
    const EARTH_RADIUS_DEFAULT: f64 = 6_371_000.0; // m, mean spherical
    const HEIGHT_DEFAULT: f64 = 693_000.0; // m, Sentinel-1 nominal altitude

    let earth_radius = EARTH_RADIUS_DEFAULT;
    let height = HEIGHT_DEFAULT;

    // Heading: approximate values for Sentinel-1 (ISCE2 computes from orbit vectors)
    let heading: f64 = if orbit_direction == "ASCENDING" {
        -13.6
    } else {
        -166.4
    };

    // Assemble the rsc content, matching the field set in the example data.rsc
    let rsc = format!(
        "\
ALOOKS                    {alooks}
ANTENNA_SIDE              -1
AZIMUTH_PIXEL_SIZE        {azimuth_pixel_size_ml}
CENTER_LINE_UTC           {center_line_utc}
EARTH_RADIUS              {earth_radius}
FILE_LENGTH               {length}
HEADING                   {heading}
HEIGHT                    {height}
LENGTH                    {length}
NCORRLOOKS                {ncorrlooks}
ORBIT_DIRECTION           {orbit_direction}
PLATFORM                  {platform}
POLARIZATION              {polarization_uc}
PRF                       {prf}
PROCESSOR                 isce
RANGE_PIXEL_SIZE          {range_pixel_size_ml}
RLOOKS                    {rlooks}
STARTING_RANGE            {starting_range}
WAVELENGTH                {wavelength}
WIDTH                     {width}
altitude                  {height}
azimuthPixelSize          {azimuth_pixel_size_ml}
azimuthResolution         {azimuth_resolution}
beam_mode                 IW
beam_swath                {beam_swath}
earthRadius               {earth_radius}
orbitNumber               {orbit_number}
passDirection             {orbit_direction}
polarization              {polarization_lc}
prf                       {prf}
radarWavelength           {wavelength}
rangePixelSize            {range_pixel_size_ml}
rangeResolution           {range_resolution}
satelliteSpeed            {sat_speed}
startUTC                  {start_utc}
startingRange             {starting_range}
stopUTC                   {stop_utc}
swathNumber               {beam_swath}
trackNumber               {track_number}
",
        alooks = alooks,
        azimuth_pixel_size_ml = azimuth_pixel_size_ml,
        center_line_utc = center_line_utc,
        earth_radius = earth_radius,
        length = length,
        heading = heading,
        height = height,
        ncorrlooks = ncorrlooks,
        orbit_direction = orbit_direction,
        platform = platform,
        polarization_uc = ref_iw.polarization.to_uppercase(),
        polarization_lc = ref_iw.polarization.to_lowercase(),
        prf = ref_iw.prf,
        range_pixel_size_ml = range_pixel_size_ml,
        rlooks = rlooks,
        starting_range = ref_iw.starting_range,
        wavelength = ref_iw.radar_wavelength,
        width = width,
        azimuth_resolution = AZIMUTH_RESOLUTION,
        range_resolution = RANGE_RESOLUTION,
        sat_speed = SAT_SPEED,
        start_utc = start_utc,
        stop_utc = stop_utc,
        beam_swath = beam_swath,
        orbit_number = ref_iw.orbit_number,
        track_number = ref_iw.track_number,
    );

    std::fs::write(&rsc_path, &rsc)
        .with_context(|| format!("Cannot write {}", rsc_path.display()))?;

    eprintln!("[mintpy-runner] Written data.rsc: {}", rsc_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Configuration types (from specification.toml)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct Spec {
    folders: Folders,
    #[serde(rename = "mintpy")]
    mintpy: MintpyParams,
}

#[derive(Debug, serde::Deserialize)]
struct Folders {
    home: String,
}

#[derive(Debug, serde::Deserialize)]
struct MintpyParams {
    tropo: Option<String>,
}

// ---------------------------------------------------------------------------
// Valid MintPy stages (smallbaselineApp.py steps)
// ---------------------------------------------------------------------------

const VALID_STAGES: &[&str] = &[
    "load_data",
    "modify_network",
    "reference_point",
    "quick_overview",
    "correct_unwrap_error",
    "invert_network",
    "correct_lod",
    "correct_troposphere",
    "correct_topography",
    "residual_rms",
    "deramp",
    "correct_timeseries",
    "geocode",
    "google_earth",
    "hdfeos5",
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_cmd(program: &str, args: &[&str]) -> Result<ExitStatus> {
    eprintln!("[mintpy-runner] Running: {} {}", program, args.join(" "));
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("Failed to spawn {program}"))?;
    Ok(status)
}

fn job_dir() -> PathBuf {
    PathBuf::from(std::env::var("JOB_DIR").unwrap_or_else(|_| "/job".into()))
}

fn results_dir() -> PathBuf {
    PathBuf::from(std::env::var("RESULTS_DIR").unwrap_or_else(|_| "/job/results".into()))
}

fn read_spec(job_dir: &Path) -> Result<Option<Spec>> {
    let spec_path = job_dir.join("specification.toml");
    if !spec_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&spec_path)
        .with_context(|| format!("Cannot read {}", spec_path.display()))?;
    let spec: Spec =
        toml::from_str(&raw).with_context(|| format!("Cannot parse {}", spec_path.display()))?;
    Ok(Some(spec))
}

// ---------------------------------------------------------------------------
// Stage execution
// ---------------------------------------------------------------------------

/// Run a single smallbaselineApp.py step, outputting results to results_dir.
fn run_stage(stage: &str, job_dir: &Path, results_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(results_dir)
        .with_context(|| format!("Cannot create {}", results_dir.display()))?;

    let results_str = results_dir.to_string_lossy();
    let status = run_cmd(
        "smallbaselineApp.py",
        &["--dir", &results_str, "--dostep", stage],
    )?;

    if !status.success() {
        bail!(
            "smallbaselineApp.py --dostep {} exited with code {:?}",
            stage,
            status.code()
        );
    }

    Ok(())
}

/// Stage load_data additionally:
///   1. Generates `{job}/reference/data.rsc` from IW*.xml (no ISCE install needed).
///   2. Generates the smallbaselineApp.cfg from specification.toml.
fn run_load_data(spec: &Option<Spec>, job_dir: &Path, results_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(results_dir)
        .with_context(|| format!("Cannot create {}", results_dir.display()))?;

    // Build data.rsc from the ISCE2 IW*.xml files so MintPy finds STARTING_RANGE
    // and all other required metadata without needing ISCE installed.
    let reference_dir = job_dir.join("reference");
    let geom_dir = job_dir.join("merged").join("geom_reference");
    build_data_rsc(&reference_dir, &geom_dir)
        .context("Failed to build data.rsc from IW*.xml files")?;

    let cfg_path = results_dir.join("smallbaselineApp.cfg");

    // Only write config if it doesn't already exist (idempotent re-runs)
    if !cfg_path.exists() {
        let tropo_method = spec
            .as_ref()
            .and_then(|s| s.mintpy.tropo.as_deref())
            .unwrap_or("height_correlation");

        let cfg = format!(
            r#"## Auto-generated by mintpy-runner — do not edit manually.
## Re-generated on each load_data invocation if absent.

mintpy.load.processor        = isce
mintpy.load.unwFile          = {job}/merged/interferograms/*/filt_*.unw
mintpy.load.corFile          = {job}/merged/interferograms/*/filt_*.cor
mintpy.load.connCompFile     = {job}/merged/interferograms/*/filt_*.unw.conncomp
mintpy.load.intFile          = None
mintpy.load.demFile          = {job}/merged/geom_reference/hgt.rdr
mintpy.load.lookupYFile      = {job}/merged/geom_reference/lat.rdr
mintpy.load.lookupXFile      = {job}/merged/geom_reference/lon.rdr
mintpy.load.incAngleFile     = {job}/merged/geom_reference/incLocal.rdr
mintpy.load.azAngleFile      = {job}/merged/geom_reference/azLocal.rdr
mintpy.load.shadowMaskFile   = {job}/merged/geom_reference/shadowMask.rdr

mintpy.troposphericDelay.method = {tropo}

mintpy.save.hdfEos5          = yes
mintpy.save.hdfEos5.update   = yes
"#,
            job = job_dir.display(),
            tropo = tropo_method,
        );

        std::fs::write(&cfg_path, cfg)
            .with_context(|| format!("Cannot write {}", cfg_path.display()))?;

        eprintln!("[mintpy-runner] Written config: {}", cfg_path.display());
    } else {
        eprintln!(
            "[mintpy-runner] Config exists, skipping generation: {}",
            cfg_path.display()
        );
    }

    run_stage("load_data", job_dir, results_dir)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn dispatch(stage: &str, job_dir: &Path, results_dir: &Path) -> Result<()> {
    if !VALID_STAGES.contains(&stage) {
        bail!(
            "Unknown stage '{}'. Valid stages: {}",
            stage,
            VALID_STAGES.join(", ")
        );
    }

    let spec = read_spec(job_dir)?;

    if stage == "load_data" {
        run_load_data(&spec, job_dir, results_dir)
    } else {
        run_stage(stage, job_dir, results_dir)
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn print_help() {
    eprintln!("mintpy-runner <stage_name>");
    eprintln!();
    eprintln!("Valid stages:");
    for s in VALID_STAGES {
        eprintln!("  {s}");
    }
    eprintln!();
    eprintln!("Environment variables:");
    eprintln!("  JOB_DIR      ISCE2 job directory  (default: /job)");
    eprintln!("  RESULTS_DIR  MintPy output dir    (default: /job/results)");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_help();
        std::process::exit(1);
    }

    let stage = &args[1];
    let job_dir = job_dir();
    let results_dir = results_dir();

    eprintln!("[mintpy-runner] Stage      : {stage}");
    eprintln!("[mintpy-runner] Job dir    : {}", job_dir.display());
    eprintln!("[mintpy-runner] Results dir: {}", results_dir.display());

    if let Err(e) = dispatch(stage, &job_dir, &results_dir) {
        eprintln!("[mintpy-runner] ERROR: {e:#}");
        std::process::exit(1);
    }

    eprintln!("[mintpy-runner] Stage '{stage}' completed successfully.");
}
