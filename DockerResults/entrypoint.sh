#!/usr/bin/env bash
# sisar/results — entrypoint
# Usage:
#   velocity
#   timeseries <lat> <lon>
#
# Placeholders (not yet implemented):
#   ai_summary
#   dem
#   3d_model

set -euo pipefail

# --------------------------------------------------------------------------- #
# Helpers                                                                      #
# --------------------------------------------------------------------------- #

die() {
    echo "[sisar/results] ERROR: $*" >&2
    exit 1
}

usage() {
    cat >&2 <<EOF
Usage:
  velocity
  timeseries <lat> <lon>

Placeholders (not yet implemented):
  ai_summary
  dem
  3d_model
EOF
    exit 1
}

# --------------------------------------------------------------------------- #
# Argument validation                                                          #
# --------------------------------------------------------------------------- #

[[ $# -lt 1 ]] && usage

RESULT_TYPE="$1"
shift

# --------------------------------------------------------------------------- #
# Paths                                                                        #
# --------------------------------------------------------------------------- #

JOB_DIR="${JOB_DIR:-/job}"
SERIES_DIR="${JOB_DIR}/series/geo"
DEM_FILE="${JOB_DIR}/dem/full_res.dem.wgs84"
RESULTS_DIR="${JOB_DIR}/results"

mkdir -p "${RESULTS_DIR}"

# --------------------------------------------------------------------------- #
# Dispatch                                                                     #
# --------------------------------------------------------------------------- #

case "${RESULT_TYPE}" in

  # -------------------------------------------------------------------------
  velocity)
    [[ $# -ne 0 ]] && die "'velocity' takes no additional arguments"

    VELOCITY_H5="${SERIES_DIR}/geo_velocity.h5"
    MASK_H5="${SERIES_DIR}/geo_maskTempCoh.h5"
    OUTPUT="${RESULTS_DIR}/velocity.png"

    [[ -f "${VELOCITY_H5}" ]] || die "File not found: ${VELOCITY_H5}"
    [[ -f "${MASK_H5}" ]]    || die "File not found: ${MASK_H5}"
    [[ -f "${DEM_FILE}" ]]   || die "File not found: ${DEM_FILE}"

    echo "[sisar/results] Generating velocity map → ${OUTPUT}" >&2

    exec view.py "${VELOCITY_H5}" velocity \
        -m "${MASK_H5}" \
        -d "${DEM_FILE}" \
        -v -10 10 \
        --notitle \
        --nodisplay \
        -o "${OUTPUT}"
    ;;

  # -------------------------------------------------------------------------
  timeseries)
    [[ $# -ne 2 ]] && die "'timeseries' requires exactly two arguments: <lat> <lon>"

    LAT="$1"
    LON="$2"

    # Basic numeric validation
    [[ "${LAT}" =~ ^-?[0-9]+(\.[0-9]+)?$ ]] || die "Invalid latitude: ${LAT}"
    [[ "${LON}" =~ ^-?[0-9]+(\.[0-9]+)?$ ]] || die "Invalid longitude: ${LON}"

    TIMESERIES_H5="${SERIES_DIR}/geo_timeseries_ERA5_ramp_demErr.h5"
    OUTPUT="${RESULTS_DIR}/series_${LAT}_${LON}.pdf"

    [[ -f "${TIMESERIES_H5}" ]] || die "File not found: ${TIMESERIES_H5}"

    echo "[sisar/results] Generating time-series for (${LAT}, ${LON}) → ${OUTPUT}" >&2

    exec tsview.py "${TIMESERIES_H5}" \
        --zf \
        -u cm \
        --lalo "${LAT}" "${LON}" \
        --nodisplay \
        -o "${OUTPUT}"
    ;;

  # -------------------------------------------------------------------------
  # Placeholders — defined but not yet implemented
  ai_summary | dem | 3d_model)
    die "Result type '${RESULT_TYPE}' is not yet implemented."
    ;;

  # -------------------------------------------------------------------------
  *)
    die "Unknown result type: '${RESULT_TYPE}'"
    ;;

esac
