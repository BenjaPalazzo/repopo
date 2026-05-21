#!/bin/bash
# =============================================================================
# entrypoint.sh — sisar/isce2 processing container
#
# Ensures the ISCE2 environment is set, then delegates to isce-runner.
#
# SISAR scheduler interface (spec §7):
#   docker run --rm -v /host/job:/job --network none \
#     sisar/isce2:latest <snake_case_stage> [--jobs N]
#
# No credentials are configured here: this container runs with --network none
# and performs no downloads. Orbit, DEM, and SLC data are expected to already
# be present in the job working directory when this container starts.
# =============================================================================

set -euo pipefail

# ISCE2 environment (Dockerfile ENV directives already set these; safety fallback)
export ISCE_ROOT=/usr/local
export ISCE_HOME=/usr/local/isce
export ISCE_STACK=${ISCE_HOME}/components/contrib/stack
export PATH="${ISCE_STACK}/topsStack:${ISCE_HOME}/bin:${ISCE_HOME}/applications:${PATH}"
export PYTHONPATH="${ISCE_ROOT}:${ISCE_HOME}/applications:${ISCE_HOME}/components:${ISCE_STACK}:${ISCE_STACK}/topsStack"

if [[ $# -eq 0 ]]; then
    echo "[entrypoint] ERROR: no stage argument provided." >&2
    exec isce-runner --help
fi

echo "[entrypoint] stage=${1} args=${*}"
exec isce-runner "$@"
