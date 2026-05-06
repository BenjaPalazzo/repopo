# ============================================================
# Dockerfile – MintPy 1.6.2
# Base:    Ubuntu 22.04 LTS + Python 3.10
# ============================================================

FROM ubuntu:22.04

# ── Evitar prompts interactivos durante apt ──────────────────
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=America/Argentina/Mendoza

# ── Variables de entorno de MintPy ──────────────────────────
ENV MINTPY_HOME=/opt/MintPy
ENV PATH="${MINTPY_HOME}/src/mintpy/cli:${PATH}"
ENV PYTHONPATH="${MINTPY_HOME}/src:${PYTHONPATH}"

# Carpeta para datos ERA5 (montar como volumen en producción)
ENV WEATHER_DIR=/era5

# ── 1. Dependencias del sistema ──────────────────────────────
RUN apt-get update && apt-get install -y --no-install-recommends \
    # herramientas base
    curl \
    wget \
    git \
    ca-certificates \
    unzip \
    # compiladores y libs C
    build-essential \
    gfortran \
    # Python 3.10 + pip
    python3 \
    python3-dev \
    python3-pip \
    python3-venv \
    # GDAL / geoespaciales
    gdal-bin \
    libgdal-dev \
    python3-gdal \
    # HDF5
    libhdf5-dev \
    # FFTW (necesario para algunas rutinas de phase-unwrapping)
    libfftw3-dev \
    libfftw3-3 \
    # NetCDF (ERA5 / PyAPS)
    libnetcdf-dev \
    # visualización headless
    python3-matplotlib \
    # utilidades extra
    less \
    vim \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# ── 2. Actualizar pip ────────────────────────────────────────
RUN python3 -m pip install --upgrade pip setuptools wheel

# ── 3. Dependencias Python base (científicas) ────────────────
RUN pip3 install --no-cache-dir \
    numpy \
    scipy \
    matplotlib \
    "gdal==$(gdal-config --version)" \
    h5py \
    netCDF4 \
    pyproj \
    shapely \
    scikit-image \
    joblib \
    lxml \
    requests \
    tqdm

# ── 4. Instalar MintPy 1.6.2 desde PyPI ─────────────────────
RUN pip3 install --no-cache-dir "MintPy==1.6.2"

# ── 5. Dependencias adicionales de MintPy ────────────────────
#    (algunas no se instalan automáticamente como dependencias)
RUN pip3 install --no-cache-dir \
    "pyaps3>=0.3.3" \
    "cartopy>=0.21" \
    "pykml" \
    "pyyaml" \
    "rich" \
    "pandas" \
    "utm" \
    "simplekml" \
    "multitaper"

# ── 6. Snaphu (phase unwrapping) ─────────────────────────────
#    MintPy llama a snaphu como binario externo
RUN apt-get update && apt-get install -y --no-install-recommends snaphu \
    && apt-get clean && rm -rf /var/lib/apt/lists/* \
    || pip3 install --no-cache-dir snaphu

# ── 7. Carpetas de trabajo ───────────────────────────────────
RUN mkdir -p /era5 /data /output

# ── 8. Verificación de la instalación ───────────────────────
RUN python3 -c "\
import mintpy; \
print('MintPy version:', mintpy.__version__)" \
    && echo "✔  MintPy instalado correctamente"

# ── 9. Directorio de trabajo por defecto ─────────────────────
WORKDIR /data

# ── 10. Entrypoint ───────────────────────────────────────────
# Por defecto muestra la ayuda de smallbaselineApp.py
# En producción se sobreescribe con: docker run ... smallbaselineApp.py config.txt
CMD ["smallbaselineApp.py", "--help"]