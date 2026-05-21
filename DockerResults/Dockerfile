# sisar/results — Result image generation container
# Version: 0.2.0
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
    libfreetype6 \
    fonts-dejavu-core \
    # Required for PDF → PNG conversion
    poppler-utils \
    && rm -rf /var/lib/apt/lists/*

# --------------------------------------------------------------------------- #
# Python packages                                                              #
# --------------------------------------------------------------------------- #

RUN GDAL_VERSION=$(gdal-config --version) && \
    pip3 install --no-cache-dir "gdal==${GDAL_VERSION}"

COPY requirements.txt /tmp/requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/requirements.txt

# --------------------------------------------------------------------------- #
# Force non-interactive matplotlib backend                                     #
# --------------------------------------------------------------------------- #
ENV MPLBACKEND=Agg

# --------------------------------------------------------------------------- #
# sisar-results binary (pre-compiled, copy-paste deployment)                  #
#                                                                              #
# Build on the host with:                                                      #
#   cargo build --release --manifest-path DockerResults/Cargo.toml            #
# Then copy the binary here before building the image:                        #
#   cp target/release/sisar-results DockerResults/sisar-results               #
# --------------------------------------------------------------------------- #
COPY sisar-results /usr/local/bin/sisar-results
RUN chmod +x /usr/local/bin/sisar-results

WORKDIR /job

ENTRYPOINT ["/usr/local/bin/sisar-results"]