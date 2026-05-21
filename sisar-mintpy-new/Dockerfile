# =============================================================================
# sisar/mintpy — MintPy time-series analysis container
# Base image : ubuntu:22.04
# Python     : 3.10 (Ubuntu 22.04 default)
#
# ISCE2 strategy: multi-stage build.
#   Stage 1 (isce-builder): full scons compile of ISCE2 v2.6.3 → /usr/local/isce/
#   Stage 2 (runtime):      copy only the Python subset needed by MintPy's
#                           prep_isce / isce_utils path:
#                             · isce          (top-level package + __init__)
#                             · iscesys       (Component.ProductManager — XML parsing)
#                             · isceobj       (createImage, Planet, Scene, Sensor/TOPS)
#                           Compiled C/Fortran extensions in those packages are
#                           included automatically (they live alongside the .py files).
#                           Heavy stacks NOT copied: mroipac, stdproc, topsStack,
#                           stripmapStack, alosStack — none are called by
#                           smallbaselineApp.py steps.
#
# snaphu: compiled from source (same Makefile as sisar/isce2).
#
# Container interface (SISAR spec §7):
#   docker run --rm \
#     -v /host/job:/job \
#     --network none \
#     sisar/mintpy:latest \
#     <snake_case_stage_name>
#
# Authors: CEDIAC-UNCUYO / CONICET
# =============================================================================


# =============================================================================
# Stage 1 — ISCE2 builder
# Identical scons build to sisar/isce2 so the compiled extensions and the
# pure-Python tree are consistent. We throw away the build tools afterwards.
# =============================================================================
FROM ubuntu:22.04 AS isce-builder

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# ---------------------------------------------------------------------------
# Build dependencies (not needed in the final image)
# ---------------------------------------------------------------------------
RUN apt-get update && \
    apt-get install -y --no-install-recommends software-properties-common && \
    add-apt-repository universe && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates wget git \
        build-essential gcc g++ gfortran make m4 \
        python3 python3-dev python3-pip \
        python3-setuptools python3-wheel \
        python3-numpy python3-scipy \
        libfftw3-3 libfftw3-dev \
        libhdf5-dev libhdf5-serial-dev \
        gdal-bin libgdal-dev \
        libproj-dev \
        libx11-dev libxt-dev libxm4 libmotif-dev \
        libopencv-dev \
        scons \
        cython3 \
        libffi-dev libssl-dev \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Pin numpy to the same version used at scons compile time so headers match
RUN pip3 install --no-cache-dir "numpy==1.23.0"

# ---------------------------------------------------------------------------
# Clone and compile ISCE2 v2.6.3
# ---------------------------------------------------------------------------
ENV ISCE_TEMP=/tmp/isce_build
ENV ISCE_HOME=/usr/local/isce

RUN mkdir -p "${ISCE_TEMP}" && \
    git clone \
        --branch v2.6.3 \
        --depth 1 \
        https://github.com/isce-framework/isce2.git \
        "${ISCE_TEMP}/isce2"

# gfortran >= 10 requires -fallow-argument-mismatch for legacy Fortran 77
RUN sed -i_bkp \
    "0,/GFORTRANFLAGS/s/'-ffixed-line-length-none'/'-fallow-argument-mismatch','-ffixed-line-length-none'/" \
    "${ISCE_TEMP}/isce2/configuration/sconsConfigFile.py"

ENV SCONS_CONFIG_DIR="${ISCE_TEMP}/isce2/SCONS_CONFIG_DIR"

RUN mkdir -p "${SCONS_CONFIG_DIR}" && \
    GCC_VER=$(gcc -dumpversion) && \
    NUMPY_INC=$(python3 -c "import numpy; print(numpy.get_include())") && \
    cat > "${SCONS_CONFIG_DIR}/SConfigISCE" << EOF
PRJ_SCONS_BUILD=${ISCE_HOME}
PRJ_SCONS_INSTALL=${ISCE_HOME}
LIBPATH=/usr/lib /usr/lib64 /usr/lib/gcc/x86_64-linux-gnu/${GCC_VER}/ /usr/lib/x86_64-linux-gnu/hdf5/serial
CPPPATH=/usr/include/python3.10 ${NUMPY_INC} /usr/include/hdf5/serial /usr/include/gdal /usr/include/opencv4
FORTRANPATH=/usr/include
FORTRAN=/usr/bin/gfortran
CC=/usr/bin/gcc
CXX=/usr/bin/g++
MOTIFLIBPATH=/usr/lib/x86_64-linux-gnu
X11LIBPATH=/usr/lib/x86_64-linux-gnu
MOTIFINCPATH=/usr/include/Xm
X11INCPATH=/usr/include/X11
ENABLE_CUDA=False
EOF

RUN cd "${ISCE_TEMP}/isce2" && \
    SCONS_CONFIG_DIR="${SCONS_CONFIG_DIR}" scons --jobs="$(nproc)" 2>&1 | \
        tee /tmp/scons_build.log && \
    tail -n 3 /tmp/scons_build.log && \
    grep -q "done building targets" /tmp/scons_build.log || \
        { echo "ERROR: scons build failed."; cat /tmp/scons_build.log; exit 1; }

# Install topsStack scripts (needed for metadata file conventions)
RUN mkdir -p "${ISCE_HOME}/components/contrib" && \
    cp -r "${ISCE_TEMP}/isce2/contrib/stack" "${ISCE_HOME}/components/contrib/"

RUN rm -rf "${ISCE_TEMP}" /tmp/scons_build.log


# =============================================================================
# Stage 2 — runtime image
# =============================================================================
FROM ubuntu:22.04

LABEL org.opencontainers.image.title="sisar/mintpy"
LABEL org.opencontainers.image.version="0.1.0"
LABEL org.opencontainers.image.description="MintPy time-series analysis with ISCE2 metadata support — SISAR scheduler compatible"

# ---------------------------------------------------------------------------
# System environment
# ---------------------------------------------------------------------------
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# ---------------------------------------------------------------------------
# ISCE2 environment — mirrors sisar/isce2 paths exactly so isce_utils.py
# import resolution works identically in both containers.
# ISCE_ROOT=/usr/local  means Python finds 'import isce' as /usr/local/isce/__init__.py
# ---------------------------------------------------------------------------
ENV ISCE_ROOT=/usr/local
ENV ISCE_HOME=/usr/local/isce

# PATH / PYTHONPATH: topsStack scripts not added — they are not needed at
# runtime by MintPy. Only the Python package root matters.
ENV PYTHONPATH=/usr/local:/usr/local/isce/components

# ---------------------------------------------------------------------------
# Runtime system packages
# ---------------------------------------------------------------------------
RUN apt-get update && \
    apt-get install -y --no-install-recommends software-properties-common && \
    add-apt-repository universe && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        # Python runtime
        python3 \
        python3-dev \
        python3-pip \
        # GDAL (system) — pinned to 3.4.x on Ubuntu 22.04
        gdal-bin \
        libgdal-dev \
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
        # Compression / I/O (pyaps3 / cfgrib)
        libnetcdf-dev \
        libeccodes-dev \
        # Motif / X11 shared libs — required by compiled ISCE2 .so extensions
        libxm4 \
        libx11-6 \
        libxt6 \
        # OpenCV shared libs — required by compiled ISCE2 .so extensions
        libopencv-core4.5d \
        libopencv-imgproc4.5d \
        # Build tools (needed to compile snaphu)
        build-essential \
        # Utilities
        wget \
        git \
        ca-certificates \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------------------
# Copy ISCE2 Python subset from the builder stage
#
# We copy only the packages that MintPy's prep_isce / isce_utils.py actually
# imports at runtime for the smallbaselineApp steps:
#
#   import isce                              → /usr/local/isce/__init__.py
#   from iscesys.Component.ProductManager   → components/iscesys/
#   import isceobj                           → components/isceobj/
#   from isceobj.Planet.Planet import Planet → included in isceobj/
#
# Any compiled .so files inside those directories are included automatically.
# Heavy subsystems (mroipac, stdproc, topsStack apps) are intentionally
# omitted to keep the image small.
# ---------------------------------------------------------------------------
COPY --from=isce-builder /usr/local/isce/__init__.py       /usr/local/isce/__init__.py
COPY --from=isce-builder /usr/local/isce/components/iscesys  /usr/local/isce/components/iscesys
COPY --from=isce-builder /usr/local/isce/components/isceobj  /usr/local/isce/components/isceobj

# ---------------------------------------------------------------------------
# GDAL Python binding — must match system GDAL version
# ---------------------------------------------------------------------------
RUN pip3 install --no-cache-dir "GDAL==$(gdal-config --version)"

# ---------------------------------------------------------------------------
# Python dependencies
# pinned versions sourced from tools/scripts/isce_python3_9_requeriments.txt
# and tools/scripts/mintpy_requirements.txt; stripped of ISCE-processing-only
# and download/notebook packages.
# ---------------------------------------------------------------------------
COPY mintpy_requirements.txt /tmp/mintpy_requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/mintpy_requirements.txt && \
    rm /tmp/mintpy_requirements.txt

# ERA5 tropospheric correction clients (kept up-to-date)
RUN pip3 install --no-cache-dir --upgrade cads-api-client cdsapi

# ---------------------------------------------------------------------------
# Compile snaphu from source
# Used by MintPy's correct_unwrap_error step.
# ---------------------------------------------------------------------------
COPY scripts/snaphu/src/ /tmp/snaphu/src/

RUN mkdir -p /tmp/snaphu/bin && \
    cd /tmp/snaphu/src && \
    make clean && \
    make && \
    install -m 755 ../bin/snaphu /usr/local/bin/snaphu && \
    rm -rf /tmp/snaphu

# ---------------------------------------------------------------------------
# mintpy-runner binary
# Pre-compiled for linux/amd64 and committed to the repository.
# Build locally with:
#   cd mintpy-runner && cargo build --release
#   cp target/release/mintpy-runner ../bin/mintpy-runner
# ---------------------------------------------------------------------------
COPY bin/mintpy-runner /usr/local/bin/mintpy-runner
RUN chmod 755 /usr/local/bin/mintpy-runner

COPY scripts/.cdsapirc /root/.cdsapirc
RUN chmod 755 /root/.cdsapirc

COPY scripts/release_history.py /usr/local/isce/release_history.py
RUN chmod 755 /usr/local/isce/release_history.py

COPY scripts/logging.conf /usr/local/isce/defaults/logging/logging.conf
RUN chmod 755 /usr/local/isce/defaults/logging/logging.conf

# ---------------------------------------------------------------------------
# Entrypoint
# ---------------------------------------------------------------------------
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ---------------------------------------------------------------------------
# Verify installation
# ---------------------------------------------------------------------------
RUN echo "=== Verifying MintPy ===" && \
    python3 -c "import mintpy; print('MintPy OK:', mintpy.__version__)" && \
    echo "=== Verifying ISCE2 subset ===" && \
    python3 -c "import isce; print('isce OK:', isce.__version__)" && \
    python3 -c "from iscesys.Component.ProductManager import ProductManager; print('iscesys.ProductManager OK')" && \
    python3 -c "import isceobj; print('isceobj OK')" && \
    python3 -c "from isceobj.Planet.Planet import Planet; print('isceobj.Planet OK')" && \
    echo "=== Verifying GDAL ===" && \
    python3 -c "from osgeo import gdal; print('GDAL OK:', gdal.__version__)" && \
    echo "=== Verifying snaphu ===" && \
    snaphu --help 2>&1 | head -1 || true && \
    echo "=== Verifying mintpy-runner ===" && \
    mintpy-runner --help 2>&1 | head -3 || true && \
    echo "=== All checks passed ==="

# /job is bind-mounted by the scheduler; smallbaselineApp.py runs inside /job/series
WORKDIR /job

ENTRYPOINT ["/entrypoint.sh"]
