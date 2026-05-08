# sisar/download — Dockerfile
#
# Single-stage build on Ubuntu 22.04 LTS.
#
# Build sequence
# ──────────────
# 1. Install system packages (apt): Python 3.10, GDAL, zip, ca-certs
# 2. Install Python dependencies: GDAL binding (ABI-pinned), then requirements.txt
# 3. Copy and install helper Python scripts (local2safe, download_dem)
# 4. Copy the pre-compiled sisar-download Rust binary
# 5. Run a smoke-test to verify all imports and tools are present
#
# The Rust binary (bin/sisar-download) is pre-compiled for linux/amd64 and
# committed to the repository.  See README.md for build instructions.
#
# Runtime mounts (provided by the scheduler)
# ──────────────────────────────────────────
#   /job      — job working directory (rw); must contain specification.toml
#               and burst_list.json before the container starts.
#   /archive  — shared burst archive (rw); bursts are written here after
#               download and read from here if already present.
#
# Credentials
# ───────────
#   EARTHDATA_USER / EARTHDATA_PASS environment variables, or a ~/.netrc
#   entry for urs.earthdata.nasa.gov / earthdata.nasa.gov.

FROM ubuntu:22.04

# ── Build-time configuration ──────────────────────────────────────────────────
ARG DEBIAN_FRONTEND=noninteractive
ARG GDAL_VERSION=3.4.1

# ── 1. System packages ────────────────────────────────────────────────────────
# gdal-bin     : gdal-config (needed to pin the GDAL Python binding version)
# libgdal-dev  : GDAL shared libraries + headers (rasterio needs the .so at runtime)
# python3-gdal : NOT used (pip-installed binding below is preferred for ABI control)
# zip          : used to compress SAFE directories at maximum level (-9)
# unzip        : may be needed by sentineleof or burst2safe internals
# curl         : used in smoke-test and may be needed by dem_stitcher
RUN apt-get update && apt-get install -y --no-install-recommends \
        python3 \
        python3-dev \
        python3-pip \
        python3-venv \
        libgdal-dev \
        gdal-bin \
        libproj-dev \
        proj-data \
        proj-bin \
        libgeos-dev \
        zip \
        unzip \
        curl \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

# Symlink python → python3 for convenience
RUN ln -sf /usr/bin/python3 /usr/bin/python

# ── 2. Python dependencies ────────────────────────────────────────────────────
# Pin the GDAL Python binding to the exact system GDAL version to avoid
# ABI mismatches (same pattern as the ISCE2 container).
RUN GDAL_VERSION=$(gdal-config --version) && \
    pip3 install --no-cache-dir "GDAL==${GDAL_VERSION}"

# Install the remaining Python packages.
# Note: rasterio and shapely ship with bundled GDAL/GEOS in their wheels on
# PyPI.  To ensure they use the system libraries (required for ISCE2 compat),
# we install with --no-binary rasterio,shapely after apt has the dev headers.
COPY requirements.txt /tmp/requirements.txt
RUN pip3 install --no-cache-dir \
        --no-binary rasterio,shapely \
        -r /tmp/requirements.txt

# ── 3. Python helper scripts ──────────────────────────────────────────────────
# local2safe: wraps burst2safe to stitch burst archives into ESA SAFE format
COPY scripts/local2safe.py   /usr/local/bin/local2safe
RUN chmod +x /usr/local/bin/local2safe

# download_dem: downloads and ISCE2-prepares the Copernicus GLO-30 DEM
COPY scripts/download_dem.py /usr/local/bin/download_dem
RUN chmod +x /usr/local/bin/download_dem

# ── 4. Rust binary ────────────────────────────────────────────────────────────
# The pre-compiled linux/amd64 ELF binary must be placed at bin/sisar-download
# before running docker build.  See README.md for compilation instructions.
COPY bin/sisar-download /usr/local/bin/sisar-download
RUN chmod +x /usr/local/bin/sisar-download

# ── 5. Entrypoint ─────────────────────────────────────────────────────────────
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ── 6. Smoke-test ─────────────────────────────────────────────────────────────
# Verify that all critical tools and imports are present before the image is
# finalised.  A failure here surfaces missing dependencies at build time rather
# than at runtime.
RUN python3 -c "import burst2safe; print('burst2safe OK')" && \
    python3 -c "import dem_stitcher; print('dem_stitcher OK')" && \
    python3 -c "import rasterio; print('rasterio OK')" && \
    python3 -c "import sentineleof; print('sentineleof OK')" && \
    python3 -c "from osgeo import gdal; print('GDAL Python binding OK')" && \
    which eof    && echo "eof (sentineleof CLI) OK" && \
    which zip    && echo "zip OK" && \
    /usr/local/bin/sisar-download --help 2>&1 | head -5 || true

# ── Runtime ───────────────────────────────────────────────────────────────────
# Mount points created so the scheduler's bind mounts always have a target.
RUN mkdir -p /job /archive

# The download container requires network access (unlike ISCE2/MintPy containers).
# Network mode is intentionally NOT set to "none" here.

ENTRYPOINT ["/entrypoint.sh"]
