#!/bin/bash
# =============================================================================
# entrypoint.sh — sisar/mintpy processing container
#
# Validates the stage argument, sets up the ISCE2 environment, changes into
# /job/series (required by smallbaselineApp.py), and delegates to mintpy-runner.
#
# SISAR scheduler interface (spec §7):
#   docker run --rm -v /host/job:/job --network none \
#     sisar/mintpy:latest <snake_case_stage>
#
# Expected layout inside /job at runtime:
#   /job/series/mintpy_params.cfg   — pre-generated MintPy parameter file
#   /job/series/                    — smallbaselineApp.py working directory
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# ISCE2 environment (Dockerfile ENV directives already set these; safety fallback)
# ---------------------------------------------------------------------------
export ISCE_ROOT=/usr/local
export ISCE_HOME=/usr/local/isce
export PYTHONPATH="${ISCE_ROOT}:${ISCE_HOME}/components"

# ---------------------------------------------------------------------------
# Argument check
# ---------------------------------------------------------------------------
if [[ $# -lt 1 ]]; then
    echo "[mintpy] ERROR: stage name required." >&2
    echo "Usage: docker run sisar/mintpy:latest <stage_name>" >&2
    exec mintpy-runner --help
fi

STAGE="$1"
SERIES_DIR="${JOB_DIR:-/job}/series"
CFG_FILE="${SERIES_DIR}/mintpy_params.cfg"

echo "[mintpy] stage     : ${STAGE}"
echo "[mintpy] series dir: ${SERIES_DIR}"
echo "[mintpy] config    : ${CFG_FILE}"

# ---------------------------------------------------------------------------
# Guard: series directory and config file must exist
# ---------------------------------------------------------------------------
if [[ ! -d "${SERIES_DIR}" ]]; then
    echo "[mintpy] ERROR: series directory not found: ${SERIES_DIR}" >&2
    exit 1
fi

if [[ ! -f "${CFG_FILE}" ]]; then
    echo "[mintpy] ERROR: mintpy_params.cfg not found: ${CFG_FILE}" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# smallbaselineApp.py must run from inside the series directory so that
# relative paths in mintpy_params.cfg resolve correctly.
# ---------------------------------------------------------------------------
cd "${SERIES_DIR}"
echo "[mintpy] working directory: $(pwd)"

exec mintpy-runner "${STAGE}"
