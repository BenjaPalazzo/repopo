#!/usr/bin/env python3
"""
entrypoint_dem.py — sisar/download-dem container entry point.
 
Reads /job/specification.toml, extracts the [dem.bounds] section, and calls
download_dem to produce the four ISCE2-ready files in /job/dem/:
 
    full_res.dem.wgs84          — ISCE-format binary raster (ellipsoidal heights)
    full_res.dem.wgs84.xml      — ISCE2 XML sidecar (WGS84 ellipsoidal tag)
    full_res.dem.wgs84.vrt      — GDAL VRT required by stackSentinel / topsStack
    full_res.dem.wgs84.aux.xml  — GDAL auxiliary (written automatically by rasterio)
 
This container is dispatched by the scheduler in parallel with
sisar/download-s1 or sisar/download-nisar; both share the same /job volume.
"""
 
import os
import subprocess
import sys
from pathlib import Path
 
# tomllib is in the stdlib from Python 3.11 (Ubuntu 24.04 ships 3.12).
# Fall back to tomli for older images.
try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        print(
            "[entrypoint-dem] ERROR: neither tomllib (Python ≥3.11) nor tomli "
            "is available. Upgrade Python or install tomli.",
            file=sys.stderr,
        )
        sys.exit(1)
 
# ── configuration from environment ───────────────────────────────────────────
# JOB_DIR can be overridden for local testing; the scheduler always uses /job.
 
JOB_DIR    = Path(os.environ.get("JOB_DIR", "/job"))
SPEC_PATH  = JOB_DIR / "specification.toml"
DEM_DIR    = JOB_DIR / "dem"
DEM_SCRIPT = Path("/usr/local/bin/download_dem")
 
# ── main ──────────────────────────────────────────────────────────────────────
 
def main() -> None:
    print(f"[entrypoint-dem] JOB_DIR={JOB_DIR}", flush=True)
 
    # ── 1. Read specification.toml ────────────────────────────────────────────
    if not SPEC_PATH.exists():
        print(f"[entrypoint-dem] ERROR: {SPEC_PATH} not found.", file=sys.stderr)
        sys.exit(1)
 
    with open(SPEC_PATH, "rb") as fh:
        spec = tomllib.load(fh)
 
    try:
        b     = spec["dem"]["bounds"]
        west  = float(b["west"])
        south = float(b["south"])
        east  = float(b["east"])
        north = float(b["north"])
    except (KeyError, TypeError, ValueError) as exc:
        print(
            f"[entrypoint-dem] ERROR: cannot read [dem.bounds] from {SPEC_PATH}: {exc}",
            file=sys.stderr,
        )
        sys.exit(1)
 
    # Optional: dem dataset override from spec (defaults to glo_30)
    dem_name = spec.get("dem", {}).get("dataset", "glo_30")
 
    # ── 2. Validate bounds ────────────────────────────────────────────────────
    if west >= east:
        print("[entrypoint-dem] ERROR: dem.bounds.west must be less than east.", file=sys.stderr)
        sys.exit(1)
    if south >= north:
        print("[entrypoint-dem] ERROR: dem.bounds.south must be less than north.", file=sys.stderr)
        sys.exit(1)
 
    # ── 3. Call download_dem ──────────────────────────────────────────────────
    print(
        f"[entrypoint-dem] Downloading {dem_name} DEM  "
        f"W={west} S={south} E={east} N={north}  →  {DEM_DIR}",
        flush=True,
    )
 
    cmd = [
        "python3", str(DEM_SCRIPT),
        "--bounds", str(west), str(south), str(east), str(north),
        "--output", str(DEM_DIR),
        "--dem",    dem_name,
    ]
 
    result = subprocess.run(cmd, cwd=str(JOB_DIR))
    if result.returncode != 0:
        print(
            f"[entrypoint-dem] ERROR: download_dem exited with code {result.returncode}.",
            file=sys.stderr,
        )
        sys.exit(result.returncode)
 
    # ── 4. Verify outputs ─────────────────────────────────────────────────────
    expected = [
        DEM_DIR / "full_res.dem.wgs84",
        DEM_DIR / "full_res.dem.wgs84.xml",
        DEM_DIR / "full_res.dem.wgs84.vrt",
    ]
    missing = [str(p) for p in expected if not p.exists()]
    if missing:
        print(
            f"[entrypoint-dem] ERROR: expected output file(s) not found: {missing}",
            file=sys.stderr,
        )
        sys.exit(1)
 
    print("[entrypoint-dem] DEM download complete.", flush=True)
    for p in DEM_DIR.iterdir():
        print(f"[entrypoint-dem]   {p.name}  ({p.stat().st_size:,} bytes)", flush=True)
 
 
if __name__ == "__main__":
    main()
 
 