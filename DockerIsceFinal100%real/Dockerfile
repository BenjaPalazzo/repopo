# =============================================================================
# sisar/isce2 — ISCE2 v2.6.3 · SISAR InSAR processing container
# Base: Ubuntu 22.04 LTS
#
# Scope: topsStack interferometric processing only (steps 01–16).
# Orbit download, data download, MintPy, and MiaplPy are handled by
# separate containers and are explicitly excluded here.
#
# Container interface (SISAR spec §7):
#   docker run --rm \
#     -v /host/job/dir:/job \
#     --network none \
#     sisar/isce2:latest \
#     <snake_case_stage_name> [--jobs N]
#
# Authors: CEDIAC-UNCUYO / CONICET
# =============================================================================

FROM ubuntu:22.04

LABEL org.opencontainers.image.title="sisar/isce2"
LABEL org.opencontainers.image.version="0.1.0"
LABEL org.opencontainers.image.description="ISCE2 v0.1.0 topsStack processing — SISAR scheduler compatible"

# ---------------------------------------------------------------------------
# System environment
# ---------------------------------------------------------------------------
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# ---------------------------------------------------------------------------
# ISCE2 environment variables
# ---------------------------------------------------------------------------
ENV ISCE_ROOT=/usr/local
ENV ISCE_HOME=/usr/local/isce
ENV ISCE_STACK=/usr/local/isce/components/contrib/stack

ENV PATH=/usr/local/isce/components/contrib/stack/topsStack:\
/usr/local/isce/bin:\
/usr/local/isce/applications:\
/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

ENV PYTHONPATH=/usr/local:\
/usr/local/isce/applications:\
/usr/local/isce/components:\
/usr/local/isce/components/contrib/stack:\
/usr/local/isce/components/contrib/stack/topsStack

# ---------------------------------------------------------------------------
# System packages
# ---------------------------------------------------------------------------
RUN apt-get update && \
    apt-get install -y --no-install-recommends software-properties-common && \
    add-apt-repository universe && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        # Base tools
        ca-certificates wget git \
        # C / C++ / Fortran compilation
        build-essential gcc g++ gfortran make m4 \
        # Python 3.10 (Ubuntu 22.04 default)
        python3 python3-dev python3-pip \
        python3-setuptools python3-wheel \
        # Scientific Python (apt) — compilation base
        python3-numpy python3-scipy python3-matplotlib python3-h5py \
        python3-astropy python3-pulp \
        # FFTW (required by ISCE2 correlation modules)
        libfftw3-3 libfftw3-dev \
        # HDF5
        libhdf5-dev libhdf5-serial-dev \
        # GDAL 3.4.x — headers for scons + CLI utilities
        gdal-bin libgdal-dev \
        # PROJ
        libproj-dev \
        # X11 and Motif (required by ISCE2 display modules)
        libx11-dev libxt-dev libxm4 libmotif-dev \
        # OpenCV C++ headers for scons compilation
        libopencv-dev \
        # SCons — ISCE2 build system
        scons \
        # Cython (used by some ISCE2 modules)
        cython3 \
        # Misc system libraries
        libffi-dev libssl-dev \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

RUN pip3 install --no-cache-dir --upgrade pip setuptools wheel

# ---------------------------------------------------------------------------
# Install numpy 1.23.0 via pip BEFORE compiling ISCE2
#
# scons uses python3 -c "import numpy; print(numpy.get_include())" to locate
# headers. Pinning here ensures the build version matches the runtime version.
# ---------------------------------------------------------------------------
RUN pip3 install --no-cache-dir "numpy==1.23.0"

# ---------------------------------------------------------------------------
# Clone and compile ISCE2 v2.6.3
# ---------------------------------------------------------------------------
ENV ISCE_TEMP=/tmp/isce_build

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

# Generate SConfigISCE — detects GCC version and numpy include path dynamically
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

RUN mkdir -p "${ISCE_HOME}/components/contrib" && \
    cp -r "${ISCE_TEMP}/isce2/contrib/stack" "${ISCE_HOME}/components/contrib/"

RUN rm -rf "${ISCE_TEMP}" /tmp/scons_build.log

# ---------------------------------------------------------------------------
# Compile snaphu from source
# snaphu is a core processing dependency (phase unwrapping, step 16).
# ---------------------------------------------------------------------------
COPY scripts/snaphu/src/ /tmp/snaphu/src/

RUN mkdir -p /tmp/snaphu/bin && \
    cd /tmp/snaphu/src && \
    make clean && \
    make && \
    install -m 755 ../bin/snaphu /usr/local/bin/snaphu && \
    rm -rf /tmp/snaphu

# ---------------------------------------------------------------------------
# Python dependencies (processing only — see isce2_requirements.txt)
# ---------------------------------------------------------------------------

# GDAL Python binding — must match system GDAL version
RUN pip3 install --no-cache-dir "GDAL==$(gdal-config --version)"

COPY isce2_requirements.txt /tmp/isce2_requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/isce2_requirements.txt && \
    rm /tmp/isce2_requirements.txt

# ---------------------------------------------------------------------------
# isce-runner binary
#
# Pre-compiled for linux/amd64 and committed to the repository.
# Build it locally with: cargo build --release --manifest-path isce-runner/Cargo.toml
# Then copy the output: cp isce-runner/target/release/isce-runner bin/isce-runner
# ---------------------------------------------------------------------------
COPY bin/isce-runner /usr/local/bin/isce-runner
RUN chmod 755 /usr/local/bin/isce-runner

# ---------------------------------------------------------------------------
# Verify installation
# ---------------------------------------------------------------------------
RUN echo "=== Verifying ISCE2 ===" && \
    python3 -c "import isce; print('ISCE2 OK:', isce.__version__)" && \
    echo "=== Verifying stackSentinel.py ===" && \
    which stackSentinel.py && \
    echo "=== Verifying snaphu ===" && \
    snaphu --help 2>&1 | head -1 || true && \
    echo "=== Verifying GDAL ===" && \
    python3 -c "from osgeo import gdal; print('GDAL:', gdal.VersionInfo('RELEASE_NAME'))" && \
    echo "=== Verifying isce-runner ===" && \
    isce-runner --help 2>&1 | head -3 || true && \
    echo "=== All checks passed ==="

# ---------------------------------------------------------------------------
# Entrypoint
#
# SISAR scheduler interface (spec §7):
#   The scheduler passes a single snake_case stage name, e.g.:
#       unpack_topo_reference
#       pairs_misreg --jobs 4
#       phase_unwrap
#
# The job working directory is bind-mounted at /job (read-write).
# Network is disabled for all processing stages.
# ---------------------------------------------------------------------------
WORKDIR /job

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
