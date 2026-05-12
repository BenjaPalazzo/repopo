#!/bin/bash
set -euo pipefail

# ---------------------------------------------------------------------------
# sisar/mintpy entrypoint
# Validates arguments and delegates to mintpy-runner.
#
# Usage (invoked by the SISAR scheduler):
#   <stage_name>
#
# where <stage_name> is a snake_case MintPy stage name, e.g.:
#   load_data
#   modify_network
#   reference_point
#   quick_overview
#   correct_unwrap_error
#   invert_network
#   correct_lod
#   correct_troposphere
#   correct_topography
#   residual_rms
#   deramp
#   correct_timeseries
#   geocode
#   google_earth
#   hdfeos5
# ---------------------------------------------------------------------------

if [[ $# -lt 1 ]]; then
    echo "[mintpy] ERROR: stage name required" >&2
    echo "Usage: docker run sisar/mintpy:latest <stage_name>" >&2
    exit 1
fi

STAGE="$1"
echo "[mintpy] Starting stage: ${STAGE}"
echo "[mintpy] Job dir  : ${JOB_DIR:-/job}"
echo "[mintpy] Results  : ${RESULTS_DIR:-/job/results}"

exec mintpy-runner "$@"
