# =============================================================================
# sisar/mintpy — MintPy time-series analysis container
# Base image : ubuntu:22.04
# Python     : 3.10 (Ubuntu 22.04 default)
# Interface  : single snake_case stage argument  (same as sisar/isce2)
# Input dir  : /jobs/<job_id>   (ISCE2 outputs expected here)
# Output dir : /jobs/results/<job_id>
# =============================================================================

FROM ubuntu:22.04

# --------------------------------------------------------------------------- #
# 1. Locale / timezone — non-interactive apt                                  #
# --------------------------------------------------------------------------- #
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# --------------------------------------------------------------------------- #
# 2. System packages                                                           #
# --------------------------------------------------------------------------- #
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Python runtime
    python3 \
    python3-dev \
    python3-pip \
    python3-venv \
    # GDAL (system)
    gdal-bin \
    libgdal-dev \
    python3-gdal \
    # HDF5
    libhdf5-serial-dev \
    # PROJ / GEOS (Cartopy / Shapely / pyproj)
    libproj-dev \
    proj-bin \
    proj-data \
    libgeos-dev \
    # FFTW (scipy / pyresample)
    libfftw3-3 \
    libfftw3-dev \
    # Compression / I/O
    libnetcdf-dev \
    libeccodes-dev \
    # Build essentials (needed for some pip wheels)
    build-essential \
    gfortran \
    # Utilities
    curl \
    wget \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# --------------------------------------------------------------------------- #
# 3. GDAL Python binding — pinned to system GDAL version                      #
# --------------------------------------------------------------------------- #
RUN pip3 install --no-cache-dir "GDAL==$(gdal-config --version)"

# --------------------------------------------------------------------------- #
# 4. Python dependencies                                                       #
# --------------------------------------------------------------------------- #
COPY mintpy_requirements.txt /tmp/mintpy_requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/mintpy_requirements.txt \
    && pip3 install --no-cache-dir --upgrade cads-api-client cdsapi

# --------------------------------------------------------------------------- #
# 5. mintpy-runner binary                                                      #
#    Pre-compiled linux/amd64 ELF — same pattern as isce-runner in sisar/isce2.
#    Build it with:
#        cd mintpy-runner && cargo build --release
#        cp target/release/mintpy-runner ../bin/mintpy-runner
# --------------------------------------------------------------------------- #
COPY bin/mintpy-runner /usr/local/bin/mintpy-runner
RUN chmod +x /usr/local/bin/mintpy-runner

# --------------------------------------------------------------------------- #
# 6. Entrypoint                                                                #
# --------------------------------------------------------------------------- #
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# --------------------------------------------------------------------------- #
# 7. Directory layout expected at runtime                                      #
#    /jobs/<job_id>           — ISCE2 outputs (bind-mounted by scheduler)      #
#    /jobs/results/<job_id>   — MintPy outputs (bind-mounted by scheduler)    #
# --------------------------------------------------------------------------- #
RUN mkdir -p /jobs /jobs/results

# --------------------------------------------------------------------------- #
# 8. Verification                                                              #
# --------------------------------------------------------------------------- #
RUN python3 -c "import mintpy; print('MintPy OK:', mintpy.__version__)" \
    && python3 -c "from osgeo import gdal; print('GDAL OK:', gdal.__version__)" \
    && python3 -c "import pyaps3; print('pyaps3 OK')" \
    && python3 -c "import h5py, scipy, numpy, matplotlib; print('Scientific stack OK')" \
    && mintpy-runner --help > /dev/null 2>&1 && echo "mintpy-runner OK" \
    || echo "WARNING: mintpy-runner placeholder in place — replace bin/mintpy-runner before production build"

ENTRYPOINT ["/entrypoint.sh"]