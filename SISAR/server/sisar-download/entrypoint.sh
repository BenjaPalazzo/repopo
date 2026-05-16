#!/usr/bin/env bash
# entrypoint.sh — sisar/download container entry point
#
# Simply delegates to the sisar-download Rust binary which orchestrates
# all download and pre-processing steps.  Exists as a thin wrapper so that:
#   1. Environment variables can be validated before the binary starts.
#   2. An interactive shell can be opened easily for debugging:
#        docker run --rm -it --entrypoint /bin/bash sisar/download:latest

set -euo pipefail

# Validate that at least one credential source is available.
# The binary also checks this, but failing early here gives a cleaner message.
if [[ -z "${EARTHDATA_USER:-}" || -z "${EARTHDATA_PASS:-}" ]]; then
    if [[ ! -f "${HOME:-/root}/.netrc" ]]; then
        echo "[entrypoint] WARNING: Neither EARTHDATA_USER/EARTHDATA_PASS env vars" >&2
        echo "[entrypoint]   nor ~/.netrc found. Downloads will likely fail." >&2
    fi
fi

echo "[entrypoint] Starting sisar-download …" >&2
exec /usr/local/bin/sisar-download "$@"
