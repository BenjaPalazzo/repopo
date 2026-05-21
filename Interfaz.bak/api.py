"""
api.py — Cliente HTTP del bot ISAT hacia el servidor SISAR.

Endpoints cubiertos:
  POST /auth/telegram          → registrar/recuperar usuario, devuelve UUID
  POST /jobs                   → enviar un job
  GET  /jobs?user_id=<uuid>    → listar jobs del usuario
  GET  /jobs/<id>              → estado de un job
  DELETE /jobs/<id>            → cancelar un job
  GET  /jobs/<id>/velocity     → imagen de mapa de velocidades (bytes)
  GET  /jobs/<id>/timeseries   → imagen/csv de serie temporal (bytes)
"""

import os
import httpx
from uuid import UUID
from config import logger

# ──────────────────────────────────────────────
# Config
# ──────────────────────────────────────────────

_IP     = os.getenv("IP", "localhost")
_PUERTO = os.getenv("PUERTO", "8000")
_TOKEN  = os.getenv("SERVER_TOKEN", "")          # Bearer token del bot como interfaz

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
        return r.json()   # { job_id, work_dir, path }


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
        return r.json()   # lista de JobResponse


async def cancelar_job(job_id: str) -> bool:
    """
    DELETE /jobs/<id>
    Cancela un job. Devuelve True si tuvo éxito (204), False si ya estaba en
    estado terminal (400/409).
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
# Resultados
# ──────────────────────────────────────────────

async def obtener_velocidad(job_id: str) -> bytes:
    """
    GET /jobs/<id>/velocity
    Devuelve el PNG del mapa de velocidades como bytes.
    """
    async with httpx.AsyncClient(timeout=60) as client:
        r = await client.get(f"{BASE_URL}/jobs/{job_id}/velocity", headers=_headers())
        _raise(r)
        return r.content


async def obtener_timeseries(
    job_id: str,
    lat: float,
    lon: float,
    formato: str = "png",
) -> bytes:
    """
    GET /jobs/<id>/timeseries?lat=&lon=&format=
    Devuelve PNG o CSV como bytes.
    """
    async with httpx.AsyncClient(timeout=120) as client:
        r = await client.get(
            f"{BASE_URL}/jobs/{job_id}/timeseries",
            params={"lat": lat, "lon": lon, "format": formato},
            headers=_headers(),
        )
        _raise(r)
        return r.content


# ──────────────────────────────────────────────
# Helpers internos
# ──────────────────────────────────────────────

def _raise(r: httpx.Response) -> None:
    """Lanza HTTPStatusError con el cuerpo del servidor incluido en el mensaje."""
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