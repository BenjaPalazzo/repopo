# sisar/download — Dockerfile
#
# Base: ubuntu:24.04 (Noble Numbat, LTS until 2029)
#   → Python 3.12 (default system interpreter)
#   → GDAL 3.8.x from the ubuntu:24.04 main archive (libgdal33 / libgdal-dev)
#
# No external PPA required: ubuntu:24.04 main ships a modern, self-consistent
# GDAL stack (libgdal-dev, gdal-bin, libgeos-dev, libproj-dev).
#
# Python packaging strategy:
#   A virtual environment at /opt/sisar owns all Python packages.
#   This avoids every conflict with Debian-managed Python packages:
#     • No need for --break-system-packages (PEP 668 workaround)
#     • No "Cannot uninstall X, RECORD file not found" errors
#     • pip, setuptools and wheel upgraded cleanly inside the venv
#   The venv's bin/ is prepended to PATH so `python` and `pip` resolve
#   to it throughout the rest of the build and at runtime.
#
# GDAL strategy (the historically problematic piece):
#   1. apt-get installs libgdal-dev, gdal-bin and all shared libraries.
#   2. pip (inside the venv) installs the GDAL Python binding with
#      --no-build-isolation, so it calls the system gdal-config and
#      links against the installed .so — not a bundled copy.
#   3. rasterio, Fiona and pyogrio are compiled from source (--no-binary)
#      so they also link against the system GDAL .so rather than bundling
#      their own. This prevents the "two GDALs" problem.
#
# Runtime mounts (provided by the scheduler)
# ──────────────────────────────────────────
#   /job      — job working directory (rw)
#   /archive  — shared burst archive (rw)
#
# Credentials
# ───────────
#   EARTHDATA_USER / EARTHDATA_PASS or ~/.netrc

FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive

# ── 1. System packages ────────────────────────────────────────────────────────
#
# python3-venv is required to create the virtual environment.
# All apt operations are collapsed into one RUN layer so that
# `rm -rf /var/lib/apt/lists/*` runs in the same layer, keeping the image lean.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        # --- build toolchain ---
        build-essential \
        gcc \
        g++ \
        # --- Python + venv support ---
        python3 \
        python3-dev \
        python3-venv \
        # --- GDAL: headers + CLI + shared libs (ubuntu 24.04 main) ---
        gdal-bin \
        libgdal-dev \
        # --- Geo-library system deps (version-matched with GDAL in main) ---
        libgeos-dev \
        libproj-dev \
        libsqlite3-dev \
        # --- Utilities ---
        curl \
        git \
        zip \
        unzip && \
    rm -rf /var/lib/apt/lists/*

# ── 2. Virtual environment ────────────────────────────────────────────────────
#
# All subsequent RUN steps and the runtime container use this venv.
# Prepending to PATH means `python`, `pip`, and installed CLI tools (eof, etc.)
# resolve here automatically — no `source activate` or explicit path needed.
RUN python3 -m venv /opt/sisar
ENV PATH="/opt/sisar/bin:$PATH"

# Upgrade pip, setuptools and wheel inside the venv where pip has full
# ownership of the install tree — no Debian RECORD conflicts possible.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install --upgrade pip setuptools wheel

# ── 3. Python GDAL binding — compiled against the system GDAL ─────────────────
#
# --no-build-isolation: uses the venv's numpy (installed first) and calls
#   the system gdal-config to find headers and the correct .so path.
#   Without this flag, pip would create an isolated build env that may resolve
#   gdal-config differently and produce a binding linked to the wrong library.
#
# We capture the exact GDAL version from gdal-config so the pip package
# version matches the installed shared library exactly.
#
# numpy must be installed before GDAL because the GDAL build compiles
# _gdal_array against the numpy C headers when --no-build-isolation is active.
# The numpy version installed here determines the ABI that _gdal_array targets —
# it must match what every other package in the stack resolves to at runtime.
#
# numpy 2.x is the correct choice:
#   • rasterio 1.4.x declares numpy>=2 as a hard requirement, so pip will
#     always resolve to numpy 2.x when rasterio is present — pinning 1.x
#     here only causes pip to silently upgrade it later, producing a
#     _gdal_array compiled against 1.x that crashes under the 2.x that
#     rasterio forced in.
#   • All other packages (burst2safe, dem-stitcher, asf-search, pyogrio)
#     declare bare `numpy` with no upper bound and are numpy-2-compatible.
#   • The GDAL PyPI binding itself has no numpy version constraint.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install --no-cache-dir "numpy>=2,<3" && \
    GDAL_VERSION=$(gdal-config --version) && \
    echo "Building GDAL Python binding for system GDAL ${GDAL_VERSION}" && \
    pip install \
        --no-build-isolation \
        --no-cache-dir \
        "GDAL==${GDAL_VERSION}"

# ── 4. Python geospatial stack ────────────────────────────────────────────────
#
# rasterio, Fiona, pyogrio and shapely are compiled from source (--no-binary)
# so they link against the same system GDAL/GEOS .so files rather than
# bundling their own — this prevents the "two GDALs" problem.
#
# Version choices:
#   numpy>=2,<3      — numpy 2.x; required by rasterio 1.4.x and compatible
#                       with all other packages in this stack.
#   rasterio 1.4.3   — requires numpy>=2; supports GDAL 3.6–3.9.
#   shapely 2.1.2    — satisfies burst2safe (>=2.1.0) and geopandas (>=2.0).
#   Fiona 1.9.6      — supports GDAL 3.5+ and Python 3.9+.
#   geopandas 1.1.3  — latest; uses pyogrio as default I/O backend.
#   pyproj 3.7.2     — latest; compatible with system libproj.
#   pyogrio 0.12.1   — required I/O backend for geopandas 1.x.
#   burst2safe 2.0.2 — current major release (upgraded from 1.4.5).
#   aiohttp 3.13.5   — satisfies burst2safe's >=3.12.6 requirement.
#   dem-stitcher / sentineleof — pinned to last known-good versions.

# Compile geo-libs from source against system GDAL/GEOS.
# Kept in a separate layer from the pure-Python deps below so that a change
# to e.g. sentineleof doesn't trigger a slow recompile of rasterio.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install \
        --no-cache-dir \
        --no-binary rasterio,Fiona,pyogrio,shapely \
        "rasterio==1.4.3" \
        "Fiona==1.9.6" \
        "pyogrio==0.12.1" \
        "shapely==2.1.2"

# Pure-Python and wheel-only dependencies (no GDAL linking involved).
COPY burst2safe /tmp/burst2safe
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install \
        --no-cache-dir \
        "pyproj==3.7.2" \
        "geopandas==1.1.3" \
        "lxml==5.3.0" \
        "tifffile>=2022.04.22" \
        "aiohttp==3.13.5" \
        "requests==2.32.3" \
        "python-dateutil==2.9.0" \
        "click==8.1.8" \
        "tqdm==4.67.1" \
        "boto3" \
        "affine" \
        "asf_search" \
        "dateparser!=1.1.0" \
        "dem-stitcher==2.5.13" \
        "sentineleof==0.12.0" \
        /tmp/burst2safe

# ── 5. Python helper scripts ──────────────────────────────────────────────────
COPY scripts/local2safe.py   /usr/local/bin/local2safe
RUN chmod +x /usr/local/bin/local2safe

# download_dem: downloads and ISCE2-prepares the Copernicus GLO-30 DEM
COPY scripts/download_dem.py /usr/local/bin/download_dem
RUN chmod +x /usr/local/bin/download_dem

# ── 6. Rust binary ────────────────────────────────────────────────────────────
COPY bin/sisar-download /usr/local/bin/sisar-download
RUN chmod +x /usr/local/bin/sisar-download
COPY bin/.netrc /root/.netrc
RUN chmod 600 /root/.netrc

# ── 7. Entrypoint ─────────────────────────────────────────────────────────────
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ── 8. Smoke-test ─────────────────────────────────────────────────────────────
# Verifies that all GDAL bindings are consistent (no import-time .so mismatch).
RUN python -c "import burst2safe; print('burst2safe OK')" && \
    python -c "import dem_stitcher; print('dem_stitcher OK')" && \
    python -c "import rasterio; print('rasterio OK')" && \
    python -c "import sentineleof; print('sentineleof OK')" && \
    python -c "from osgeo import gdal; print('GDAL Python binding OK, version:', gdal.__version__)" && \
    python -c "import fiona; print('Fiona OK')" && \
    python -c "import pyogrio; print('pyogrio OK')" && \
    which eof  && echo "eof (sentineleof CLI) OK" && \
    which zip  && echo "zip OK" && \
    /usr/local/bin/sisar-download --help 2>&1 | head -5 || true

# ── Runtime ───────────────────────────────────────────────────────────────────
RUN mkdir -p /job /archive

ENTRYPOINT ["/entrypoint.sh"]
