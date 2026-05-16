# sisar/download — Container

**Version:** 0.1.0  
**Base image:** Ubuntu 22.04 LTS

The download container is the first processing stage in the SISAR pipeline.
It is invoked by the scheduler (via bollard) when a job transitions from
`Queued → Downloading`.

---

## What it does

1. **Reads inputs** — `/job/burst_list.json` and `/job/specification.toml`
2. **Checks the archive** — for each burst in `burst_list.json`, verifies
   that the `.tiff` and `.xml` files exist under `/archive/`
3. **Downloads missing bursts** — fetches any absent files from the ASF
   burst extractor (`sentinel1-burst.asf.alaska.edu`) using NASA Earthdata
   credentials
4. **Stitches SAFEs** — runs `local2safe` (burst2safe) on the full
   `burst_list.json`, writing `.SAFE` directories to `/job/data/`
5. **Zips SAFEs** — compresses each `.SAFE` directory to `.zip` at
   maximum compression (`zip -r -9`) and removes the unzipped directory
6. **Downloads DEM** — fetches the Copernicus GLO-30 DEM for the bounds
   defined in `specification.toml [dem.bounds]`, writes
   `/job/dem/full_res.dem.wgs84` and its ISCE2 XML sidecar
7. **Downloads orbits** — runs `sentineleof` (`eof`) against `/job/data/`
   to download matching precise orbit files (POE) into `/job/orbits/`

---

## Mount points

| Host path | Container path | Mode | Written by |
|---|---|---|---|
| `{jobs_root}/{job_id}` | `/job` | rw | API server (spec + burst_list), this container (data, dem, orbits) |
| `{archive_root}` | `/archive` | rw | This container (caches downloaded bursts) |

---

## Credentials

NASA Earthdata credentials are required to download burst files from ASF.
Provide them as:

- **Environment variables** (preferred in Docker): `EARTHDATA_USER` and `EARTHDATA_PASS`
- **`~/.netrc`** file: add an entry for `urs.earthdata.nasa.gov` or `earthdata.nasa.gov`

```netrc
machine urs.earthdata.nasa.gov
    login <your_username>
    password <your_password>
```

---

## Directory layout after the container finishes

```
/job/
├── specification.toml          ← written by API server; unchanged
├── burst_list.json             ← written by API server; unchanged
├── data/
│   └── S1_IW_SLC_….zip        ← stitched + zipped SAFE(s)
├── dem/
│   ├── full_res.dem.wgs84      ← Copernicus GLO-30 DEM in ISCE2 format
│   └── full_res.dem.wgs84.xml  ← ISCE2 XML sidecar (WGS84 ellipsoidal)
└── orbits/
    └── S1X_OPER_AUX_POEORB_….EOF   ← precise orbit file(s)
```

---

## Building the image

```bash
# 1. Compile the Rust binary (see bin/README.md for alternatives)
cd sisar-download/
cargo build --release
cp target/release/sisar-download bin/sisar-download

# 2. Build the Docker image
docker build -t sisar/download:latest .
```

---

## Local testing with docker compose

```bash
cp env.example .env
# Edit .env: set JOB_DIR, ARCHIVE_DIR, EARTHDATA_USER, EARTHDATA_PASS

# Run the full download pipeline:
docker compose run --rm download

# Open a debug shell:
docker compose run --rm --entrypoint /bin/bash download
```

---

## Scheduler integration

The scheduler configuration (`config.toml`) must reference this image:

```toml
[containers]
download = "sisar/download:latest"
```

The container receives no arguments; all configuration is read from
`/job/specification.toml` and `/job/burst_list.json`.

Exit code `0` → scheduler advances to `IsceProcessing { UnpackTopoReference }`.  
Non-zero exit → scheduler writes `Failed { ContainerFailed { exit_code } }`.

---

## Dependency notes

| Dependency | Why |
|---|---|
| `burst2safe` | Stitches individual ASF burst files (`.tiff` + `.xml`) into an ESA SAFE directory. Requires internet at runtime only for burst download; stitching is local. |
| `dem-stitcher` | Downloads and stitches Copernicus DEM tiles. Uses GDAL/rasterio internally. |
| `sentineleof` | Downloads POE orbit files from ESA. Requires internet access. |
| `rasterio` | Used by `dem-stitcher` to write the DEM in ISCE2-compatible format. Built against system GDAL for ABI compatibility. |
| `lxml` | Tags the DEM XML sidecar with the WGS84 ellipsoidal reference required by ISCE2. |
| GDAL (system) | GDAL shared libraries; rasterio and the GDAL Python binding must match the system version. |
| `zip` | Compresses SAFE directories at maximum compression level. |

---

## Notes and conflicts

### `specification.toml [folders.dem]`

The specification uses `dem = "/job/dem/full_res.dem.wgs84"` to point to the
DEM *file*, not the directory. This is the convention expected by `isce-runner`
(the ISCE2 container) and is what `download_dem.py` produces.

### DEM bounds vs. processing bounds

`[dem.bounds]` is derived from the union of all selected burst footprints by
the API server. `[processing.bounds]` is the user's original AOI. Both are
present in `specification.toml`; this container uses only `[dem.bounds]` for
the DEM download.

### `local2safe --work_dir` vs. output location

`local2safe` places the `.SAFE` directory inside `--work_dir`. We pass
`/job/data` so all SAFEs land there before being zipped in place.

### Orbit download timing

`sentineleof` must run *after* the SAFEs are zipped, because it scans the
directory for S1 `.zip` or `.SAFE` filenames to determine acquisition times.
The zipped SAFEs are sufficient — `eof` reads the filename only, not the
archive content.
