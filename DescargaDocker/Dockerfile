# sisar/download — Dockerfile
#
# Base: ghcr.io/osgeo/gdal:ubuntu-full-3.6.4
#   → Ubuntu 22.04 + GDAL 3.6.4 preinstalado + Python 3.10.
#   → burst2safe requiere Python >=3.10 y shapely>=2.1.0
#
# Runtime mounts (provided by the scheduler)
# ──────────────────────────────────────────
#   /job      — job working directory (rw)
#   /archive  — shared burst archive (rw)
#
# Credentials
# ───────────
#   EARTHDATA_USER / EARTHDATA_PASS o ~/.netrc

FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive

# ── 1. System packages ────────────────────────────────────────────────────────
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    find /etc/apt -name "*.list" -exec sed -i \
    's|http://archive.ubuntu.com/ubuntu|http://mirrors.dc.clear.net.ar/ubuntu|g; \
    s|http://security.ubuntu.com/ubuntu|http://mirrors.dc.clear.net.ar/ubuntu|g' {} + && \
    apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    python3-dev \
    python3-pip \
    zip \
    unzip \
    curl \
    ca-certificates \
    git \
    libgdal-dev \
    libproj-dev \
    libgeos-dev \
    && rm -rf /var/lib/apt/lists/*

RUN ln -sf /usr/bin/python3 /usr/bin/python

RUN GDAL_VERSION=$(gdal-config --version) && \
    pip3 install gdal[numpy]==${GDAL_VERSION}

# ── 2. Python dependencies ────────────────────────────────────────────────────
# Dependencias derivadas de los pyproject.toml de burst2safe, dem_stitcher
# y requirements.txt de sentineleof.
#
# burst2safe  requiere: gdal, shapely>=2.1.0, numpy, lxml, tifffile>=2022.04.22,
#                       asf_search, dateparser!=1.1.0, aiohttp>=3.12.6
# dem_stitcher requiere: affine, boto3, geopandas, numpy, pyproj, rasterio,
#                        requests, shapely, tqdm
# sentineleof  requiere: click==6.7, python-dateutil>=2.5.1, requests>=2.20.0
#
# Se usan wheels precompilados para rasterio y shapely (no --no-binary)
# ya que la imagen base OSGEO provee GDAL correctamente.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip3 install --upgrade pip setuptools wheel && \
    pip3 install \
    --ignore-installed \
    "numpy==1.23.0" \
    "shapely==2.1.0" \
    "rasterio==1.3.0" \
    "pyproj==3.3.1" \
    "geopandas== 1.0.1" \
    "lxml==4.9.0" \
    "tifffile>=2022.04.22" \
    "aiohttp>=3.12.6" \
    "requests==2.28.1" \
    "python-dateutil==2.8.2" \
    "click==8.1.3" \
    "tqdm==4.64.0" \
    "boto3" \
    "affine" \
    "asf_search" \
    "dateparser!=1.1.0" \
    dem_stitcher \
    sentineleof

COPY burst2safe /tmp/burst2safe
RUN pip3 install /tmp/burst2safe
# ── 3. Python helper scripts ──────────────────────────────────────────────────
COPY scripts/local2safe.py   /usr/local/bin/local2safe
RUN chmod +x /usr/local/bin/local2safe

# download_dem: downloads and ISCE2-prepares the Copernicus GLO-30 DEM
COPY scripts/download_dem.py /usr/local/bin/download_dem
RUN chmod +x /usr/local/bin/download_dem

# ── 4. Rust binary ────────────────────────────────────────────────────────────
COPY bin/sisar-download /usr/local/bin/sisar-download
RUN chmod +x /usr/local/bin/sisar-download
COPY bin/.netrc /root/.netrc
RUN chmod 600 /root/.netrc

# ── 5. Entrypoint ─────────────────────────────────────────────────────────────
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ── 6. Smoke-test ─────────────────────────────────────────────────────────────
RUN python3 -c "import burst2safe; print('burst2safe OK')" && \
    python3 -c "import dem_stitcher; print('dem_stitcher OK')" && \
    python3 -c "import rasterio; print('rasterio OK')" && \
    python3 -c "import sentineleof; print('sentineleof OK')" && \
    python3 -c "from osgeo import gdal; print('GDAL Python binding OK')" && \
    which eof  && echo "eof (sentineleof CLI) OK" && \
    which zip  && echo "zip OK" && \
    /usr/local/bin/sisar-download --help 2>&1 | head -5 || true

# ── Runtime ───────────────────────────────────────────────────────────────────
RUN mkdir -p /job /archive

ENTRYPOINT ["/entrypoint.sh"]
