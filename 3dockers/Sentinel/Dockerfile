# sisar/download-s1 — Dockerfile
#
# Imagen especializada para descarga de datos Sentinel-1.
# Responsabilidades:
#   1. Verificar el cache en /archive y descargar bursts faltantes desde ASF
#   2. Ensamblar SAFEs con local2safe (burst2safe)
#   3. Comprimir SAFEs a .zip (layout ESA canónico para ISCE2)
#
# Las órbitas precisas corren en un contenedor separado (junto con ISCE2).
# El DEM corre en sisar/download-dem (solo para MintPy).
#
# Runtime mounts (provistos por el scheduler)
# ───────────────────────────────────────────
#   /job      — directorio de trabajo del job (rw)
#               debe contener burst_list.json y burst_stitch/*.json
#   /archive  — cache compartido de bursts (rw, persiste entre jobs)
#
# Credenciales
# ────────────
#   EARTHDATA_USER / EARTHDATA_PASS   (vars de entorno, recomendado)
#   ~/.netrc                           (alternativa)

FROM ghcr.io/osgeo/gdal:ubuntu-full-3.6.4

ARG DEBIAN_FRONTEND=noninteractive

# ── 1. Paquetes del sistema ───────────────────────────────────────────────────
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    find /etc/apt -name "*.list" -exec sed -i \
    's|http://archive.ubuntu.com/ubuntu|http://mirrors.dc.clear.net.ar/ubuntu|g; \
    s|http://security.ubuntu.com/ubuntu|http://mirrors.dc.clear.net.ar/ubuntu|g' {} + && \
    apt-get update && apt-get install -y --no-install-recommends \
    python3-dev \
    python3-pip \
    zip \
    unzip \
    curl \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN ln -sf /usr/bin/python3 /usr/bin/python

# Binding Python de GDAL (versión debe coincidir con la del sistema)
RUN GDAL_VERSION=$(gdal-config --version) && \
    pip3 install gdal[numpy]==${GDAL_VERSION}

# ── 2. Dependencias Python ────────────────────────────────────────────────────
# Solo las necesarias para S1: burst2safe y sus dependencias.
# sentineleof y dem_stitcher NO se instalan aquí.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip3 install --upgrade pip setuptools wheel && \
    pip3 install \
    --ignore-installed \
    "numpy==1.23.0" \
    "shapely==2.1.0" \
    "rasterio==1.3.0" \
    "pyproj==3.3.1" \
    "geopandas==1.0.1" \
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
    "dateparser!=1.1.0"

# burst2safe ya está disponible en PyPI. Instalamos la versión 1.4.5 que es compatible
# con las versiones de numpy y rasterio fijadas anteriormente.
RUN pip3 install burst2safe==1.4.5

# ── 3. Scripts Python ─────────────────────────────────────────────────────────
# Solo local2safe — download_dem.py va en sisar/download-dem
COPY scripts/local2safe.py /usr/local/bin/local2safe
RUN chmod +x /usr/local/bin/local2safe

# ── 4. Binario Rust ───────────────────────────────────────────────────────────
COPY bin/sisar-download-s1 /usr/local/bin/sisar-download-s1
RUN chmod +x /usr/local/bin/sisar-download-s1

# Credenciales .netrc opcionales (fallback si no se usan vars de entorno)
COPY bin/.netrc /root/.netrc
RUN chmod 600 /root/.netrc

# ── 5. Entrypoint ─────────────────────────────────────────────────────────────
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ── 6. Smoke-test de importaciones ───────────────────────────────────────────
RUN python3 -c "import burst2safe; print('burst2safe OK')" && \
    python3 -c "import rasterio; print('rasterio OK')" && \
    python3 -c "from osgeo import gdal; print('GDAL Python binding OK')" && \
    which zip  && echo "zip OK" && \
    /usr/local/bin/sisar-download-s1 --help 2>&1 | head -5 || true

# ── Runtime ───────────────────────────────────────────────────────────────────
RUN mkdir -p /job /archive

ENTRYPOINT ["/entrypoint.sh"]
