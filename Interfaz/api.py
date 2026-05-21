"""
api.py — Cliente HTTP del bot ISAT hacia el servidor SISAR.

Endpoints cubiertos:
  POST /auth/telegram              → registrar/recuperar usuario, devuelve UUID
  POST /jobs                       → enviar un job
  GET  /jobs?user_id=<uuid>        → listar jobs del usuario
  GET  /jobs/<id>                  → estado de un job
  DELETE /jobs/<id>                → cancelar un job
  POST /jobs/<id>/results          → encolar un resultado (velocity/timeseries)
  GET  /jobs/<id>/results/<req_id> → estado del resultado o bytes si completado
"""

import os
import httpx
from uuid import UUID
from config import logger
import asyncio

# ──────────────────────────────────────────────
# Config
# ──────────────────────────────────────────────

_IP     = os.getenv("IP", "localhost")
_PUERTO = os.getenv("PUERTO", "8000")
_TOKEN  = os.getenv("SERVER_TOKEN", "")

BASE_URL = f"http://{_IP}:{_PUERTO}"


def _headers() -> dict:
    return {"Authorization": f"Bearer {_TOKEN}"}


# ──────────────────────────────────────────────
# Auth
# ──────────────────────────────────────────────

async def registrar_usuario(telegram_id: int, display_name: str) -> UUID:
    """
    POST /auth/telegram
    Registra al usuario si no existe y devuelve su UUID interno.
    """
    payload = {
        "telegram_id": str(telegram_id),
        "display_name": display_name,
    }
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.post(f"{BASE_URL}/auth/telegram", json=payload, headers=_headers())
        _raise(r)
        data = r.json()
        return UUID(data["user_id"])


# ──────────────────────────────────────────────
# Jobs
# ──────────────────────────────────────────────

async def enviar_job(
    user_uuid: UUID,
    north: float,
    south: float,
    east: float,
    west: float,
    start: str,
    end: str,
    workflow: str | None = None,
    range_looks: int | None = None,
    azimuth_looks: int | None = None,
    connections: int | None = None,
    prepaid_code: str | None = None,
) -> dict:
    """
    POST /jobs
    Envía el job al servidor. Devuelve { job_id, work_dir, path }.
    """
    payload: dict = {
        "user_id": str(user_uuid),
        "north":   north,
        "south":   south,
        "east":    east,
        "west":    west,
        "start":   start,
        "end":     end,
    }
    if workflow:
        payload["workflow"] = workflow
    if range_looks is not None:
        payload["range_looks"] = range_looks
    if azimuth_looks is not None:
        payload["azimuth_looks"] = azimuth_looks
    if connections is not None:
        payload["connections"] = connections
    if prepaid_code:
        payload["prepaid_code"] = prepaid_code

    logger.info(f"POST {BASE_URL}/jobs → {payload}")

    async with httpx.AsyncClient(timeout=30) as client:
        r = await client.post(f"{BASE_URL}/jobs", json=payload, headers=_headers())
        _raise(r)
        return r.json()


async def obtener_job(job_id: str) -> dict:
    """
    GET /jobs/<id>
    Devuelve el estado actual de un job.
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.get(f"{BASE_URL}/jobs/{job_id}", headers=_headers())
        _raise(r)
        return r.json()


async def listar_jobs(user_uuid: UUID) -> list[dict]:
    """
    GET /jobs?user_id=<uuid>
    Lista todos los jobs del usuario, más recientes primero.
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.get(
            f"{BASE_URL}/jobs",
            params={"user_id": str(user_uuid)},
            headers=_headers(),
        )
        _raise(r)
        return r.json()


async def cancelar_job(job_id: str) -> bool:
    """
    DELETE /jobs/<id>
    Cancela un job. Devuelve True si tuvo éxito (204).
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.delete(f"{BASE_URL}/jobs/{job_id}", headers=_headers())
        if r.status_code == 204:
            return True
        if r.status_code in (400, 409):
            return False
        _raise(r)
        return False


# ──────────────────────────────────────────────
# Códigos prepago
# ──────────────────────────────────────────────

async def consultar_codigo(code: str) -> dict:
    """
    GET /codes/<code>
    Consulta info de un prepaid code sin consumirlo.
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.get(f"{BASE_URL}/codes/{code}", headers=_headers())
        _raise(r)
        return r.json()


# ──────────────────────────────────────────────
# Usuarios
# ──────────────────────────────────────────────

async def obtener_usuario_me(user_uuid: UUID) -> dict:
    """
    GET /users/me?user_id=<uuid>
    Devuelve { user_id, display_name, tier }.
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.get(
            f"{BASE_URL}/users/me",
            params={"user_id": str(user_uuid)},
            headers=_headers(),
        )
        _raise(r)
        return r.json()


# ──────────────────────────────────────────────
# Resultados (cola asíncrona genérica)
# ──────────────────────────────────────────────

async def encolar_resultado(
    job_id: str,
    result_type: str,
    params: dict | None = None,
) -> dict | bytes:
    """
    POST /jobs/<id>/results
    Encola la generación de un resultado.

    Para velocity: si el archivo ya está listo en el servidor devuelve bytes
    directamente (HTTP 200 con Content-Type image/png).

    Para los demás tipos: devuelve { result_request_id, state } (HTTP 202).
    El caller debe hacer polling con obtener_resultado().
    """
    payload: dict = {"result_type": result_type}
    if params:
        payload["params"] = params

    async with httpx.AsyncClient(timeout=30) as client:
        r = await client.post(
            f"{BASE_URL}/jobs/{job_id}/results",
            json=payload,
            headers=_headers(),
        )
        _raise(r)

        # Velocity fast-path: servidor devolvió la imagen directamente.
        if r.status_code == 200 and r.headers.get("content-type", "").startswith("image/"):
            return r.content

        return r.json()  # { result_request_id, state }


async def obtener_resultado(job_id: str, request_id: str) -> dict | bytes:
    """
    GET /jobs/<id>/results/<request_id>

    - Devuelve bytes si el resultado está completado (content-type image/* o similar).
    - Devuelve dict { state, ... } si sigue en cola/procesando/fallido.
    """
    async with httpx.AsyncClient(timeout=60) as client:
        r = await client.get(
            f"{BASE_URL}/jobs/{job_id}/results/{request_id}",
            headers=_headers(),
        )
        _raise(r)

        ct = r.headers.get("content-type", "")
        if ct.startswith("image/") or ct.startswith("application/octet-stream"):
            return r.content

        return r.json()

async def obtener_velocidad(job_id: str) -> bytes:
    """
    Encola y espera la generación del mapa de velocidades para un job.
    Devuelve los bytes de la imagen PNG.
    """
    resultado = await encolar_resultado(job_id, "velocity")

    # Fast-path: el servidor ya devolvió la imagen directamente
    if isinstance(resultado, bytes):
        return resultado

    # Slow-path: polling hasta que esté listo
    request_id = resultado["result_request_id"]
    while True:
        data = await obtener_resultado(job_id, request_id)
        if isinstance(data, bytes):
            return data
        if data.get("state") in ("failed", "cancelled"):
            raise RuntimeError(f"Resultado velocity falló: {data}")
        await asyncio.sleep(10)


async def obtener_timeseries(
    job_id: str,
    lat: float,
    lon: float,
    fmt: str = "png",
) -> bytes:
    """
    Encola y espera la generación de la serie temporal para un job.
    Devuelve los bytes de la imagen.
    """
    params = {"lat": lat, "lon": lon, "format": fmt}
    resultado = await encolar_resultado(job_id, "timeseries", params=params)

    if isinstance(resultado, bytes):
        return resultado

    request_id = resultado["result_request_id"]
    while True:
        data = await obtener_resultado(job_id, request_id)
        if isinstance(data, bytes):
            return data
        if data.get("state") in ("failed", "cancelled"):
            raise RuntimeError(f"Resultado timeseries falló: {data}")
        await asyncio.sleep(10)


# ──────────────────────────────────────────────
# Helper interno
# ──────────────────────────────────────────────

def _raise(r: httpx.Response) -> None:
    """Lanza HTTPStatusError con el cuerpo del servidor en el mensaje."""
    try:
        r.raise_for_status()
    except httpx.HTTPStatusError as e:
        body = ""
        try:
            body = r.json().get("error", r.text)
        except Exception:
            body = r.text
        logger.error(f"HTTP {r.status_code} ← {BASE_URL}: {body}")
        raise httpx.HTTPStatusError(
            message=f"[{r.status_code}] {body}",
            request=e.request,
            response=e.response,
        )


# ──────────────────────────────────────────────
# Demos
# ──────────────────────────────────────────────

async def listar_demos() -> list[dict]:
    """
    GET /demos
    Devuelve la lista de demos disponibles: [{ "name": "..." }, ...]
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.get(f"{BASE_URL}/demos", headers=_headers())
        _raise(r)
        return r.json().get("demos", [])


async def encolar_resultado_demo(
    demo_name: str,
    result_type: str,
    params: dict | None = None,
) -> dict | bytes:
    """
    POST /demos/:name/results
    Encola un resultado para una demo.

    Velocity fast-path: si el archivo ya está listo devuelve bytes directamente
    (HTTP 200 image/png).  Para otros tipos devuelve { result_request_id, state }
    (HTTP 202).
    """
    payload: dict = {"result_type": result_type}
    if params:
        payload["params"] = params

    async with httpx.AsyncClient(timeout=30) as client:
        r = await client.post(
            f"{BASE_URL}/demos/{demo_name}/results",
            json=payload,
            headers=_headers(),
        )
        _raise(r)

        if r.status_code == 200 and r.headers.get("content-type", "").startswith("image/"):
            return r.content

        return r.json()


async def obtener_resultado_demo(demo_name: str, request_id: str) -> dict | bytes:
    """
    GET /demos/:name/results/:request_id
    Consulta el estado de un resultado de demo, o devuelve los bytes si está listo.
    """
    async with httpx.AsyncClient(timeout=60) as client:
        r = await client.get(
            f"{BASE_URL}/demos/{demo_name}/results/{request_id}",
            headers=_headers(),
        )
        _raise(r)

        ct = r.headers.get("content-type", "")
        if ct.startswith("image/") or ct.startswith("application/octet-stream"):
            return r.content

        return r.json()


async def obtener_velocidad_demo(demo_name: str) -> bytes:
    """
    Encola y espera el mapa de velocidades de una demo.
    Devuelve los bytes de la imagen PNG.
    """
    resultado = await encolar_resultado_demo(demo_name, "velocity")

    if isinstance(resultado, bytes):
        return resultado

    request_id = resultado["result_request_id"]
    while True:
        data = await obtener_resultado_demo(demo_name, request_id)
        if isinstance(data, bytes):
            return data
        if data.get("state") in ("failed", "cancelled"):
            raise RuntimeError(f"Resultado velocity de demo falló: {data}")
        await asyncio.sleep(10)


async def obtener_timeseries_demo(
    demo_name: str,
    lat: float,
    lon: float,
    fmt: str = "png",
) -> bytes:
    """
    Encola y espera la serie temporal de una demo en las coordenadas dadas.
    Devuelve los bytes de la imagen.
    """
    params = {"lat": lat, "lon": lon, "format": fmt}
    resultado = await encolar_resultado_demo(demo_name, "timeseries", params=params)

    if isinstance(resultado, bytes):
        return resultado

    request_id = resultado["result_request_id"]
    while True:
        data = await obtener_resultado_demo(demo_name, request_id)
        if isinstance(data, bytes):
            return data
        if data.get("state") in ("failed", "cancelled"):
            raise RuntimeError(f"Resultado timeseries de demo falló: {data}")
        await asyncio.sleep(10)
