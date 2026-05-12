# sisar/results — Result image generation container
# Version: 0.1.0
# Base: Ubuntu 22.04 LTS

FROM ubuntu:22.04

# --------------------------------------------------------------------------- #
# System packages                                                              #
# --------------------------------------------------------------------------- #
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 \
    python3-pip \
    python3-dev \
    gdal-bin \
    libgdal-dev \
    libhdf5-serial-dev \
    libproj-dev \
    proj-bin \
    libgeos-dev \
    libeccodes-dev \
    # Needed by matplotlib for non-display (Agg backend)
    libfreetype6 \
    fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

# --------------------------------------------------------------------------- #
# Python packages                                                              #
# --------------------------------------------------------------------------- #

# Pin GDAL binding to the system GDAL version
RUN GDAL_VERSION=$(gdal-config --version) && \
    pip3 install --no-cache-dir "gdal==${GDAL_VERSION}"

COPY requirements.txt /tmp/requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/requirements.txt

# --------------------------------------------------------------------------- #
# Force non-interactive matplotlib backend                                     #
# --------------------------------------------------------------------------- #
ENV MPLBACKEND=Agg

# --------------------------------------------------------------------------- #
# Entrypoint                                                                   #
# --------------------------------------------------------------------------- #
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /job

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
