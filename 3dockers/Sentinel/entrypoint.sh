#!/usr/bin/env bash
# entrypoint.sh — sisar/download-s1
#
# Wrapper delgado sobre el binario Rust sisar-download-s1.
# Valida credenciales antes de arrancar y facilita el debug interactivo:
#   docker run --rm -it --entrypoint /bin/bash sisar/download-s1:latest

set -euo pipefail

# Validar que haya al menos una fuente de credenciales disponible.
if [[ -z "${EARTHDATA_USER:-}" || -z "${EARTHDATA_PASS:-}" ]]; then
    if [[ ! -f "${HOME:-/root}/.netrc" ]]; then
        echo "[entrypoint-s1] WARNING: No se encontraron EARTHDATA_USER/EARTHDATA_PASS" >&2
        echo "[entrypoint-s1]   ni ~/.netrc. Las descargas probablemente fallarán." >&2
    fi
fi

echo "[entrypoint-s1] Iniciando sisar-download-s1 …" >&2
exec /usr/local/bin/sisar-download-s1 "$@"
