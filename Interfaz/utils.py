import re
import httpx
from datetime import datetime
from config import logger, FORMATO_FECHA


# ──────────────────────────────────────────────
# UBICACIÓN
# ──────────────────────────────────────────────

def resolver_url_corta(url: str) -> str:
    """Sigue redirects y devuelve la URL final expandida."""
    try:
        with httpx.Client(follow_redirects=True, timeout=5) as client:
            response = client.get(url)
            return str(response.url)
    except Exception as e:
        logger.warning(f"No se pudo resolver la URL '{url}': {e}")
        return url


def parsear_ubicacion_texto(texto: str) -> tuple[float, float] | None:
    """
    Intenta extraer (lat, lon) de:
    - Google Maps: ?q=lat,lon  |  ?ll=lat,lon  |  /@lat,lon  |  !3dlat!4dlon
    - OpenStreetMap: #map=z/lat/lon
    - Coordenadas crudas: "-32.89, -68.84" o "-32.89 -68.84"
    """
    # Google Maps: ?q=lat,lon o ?ll=lat,lon
    m = re.search(r"[?&](?:q|ll)=(-?\d+\.?\d*),(-?\d+\.?\d*)", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    # Google Maps: /@lat,lon
    m = re.search(r"/@(-?\d+\.?\d*),(-?\d+\.?\d*)", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    # Google Maps: !3dlat!4dlon (URLs largas de lugares)
    m = re.search(r"!3d(-?\d+\.?\d*)!4d(-?\d+\.?\d*)", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    # OpenStreetMap: #map=z/lat/lon
    m = re.search(r"#map=\d+/(-?\d+\.?\d*)/(-?\d+\.?\d*)", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    # Google Maps: /search/lat,+lon (URLs resueltas de maps.app.goo.gl)
    m = re.search(r"/search/(-?\d+\.?\d*),\+?(-?\d+\.?\d*)", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    # Coordenadas crudas: "-32.89, -68.84" o "-32.89 -68.84"
    m = re.fullmatch(r"\s*(-?\d+\.?\d*)[,\s]+(-?\d+\.?\d*)\s*", texto)
    if m:
        return float(m.group(1)), float(m.group(2))

    return None


def extraer_ubicacion_de_texto(texto: str) -> tuple[float, float] | None:
    """
    Wrapper que resuelve URLs acortadas antes de parsear.
    Devuelve (lat, lon) o None.
    """
    url_match = re.search(r"https?://\S+", texto)
    if url_match:
        url_original = url_match.group()
        url_resuelta = resolver_url_corta(url_original)
        logger.info(f"URL resuelta: {url_original} → {url_resuelta}")
        texto = texto.replace(url_original, url_resuelta)

    return parsear_ubicacion_texto(texto)


# ──────────────────────────────────────────────
# FECHAS
# ──────────────────────────────────────────────

def parsear_fecha(texto: str) -> datetime | None:
    """
    Intenta parsear una fecha en formato YYYY-MM-DD.
    Devuelve un objeto datetime o None si el formato es inválido.
    """
    try:
        return datetime.strptime(texto.strip(), FORMATO_FECHA)
    except ValueError:
        return None


def validar_rango_fechas(inicio: datetime, fin: datetime) -> str | None:
    """
    Valida que el rango de fechas sea coherente.
    Devuelve un mensaje de error o None si todo está bien.
    """
    if fin < inicio:
        return "❌ La fecha de fin no puede ser anterior a la de inicio."
    if (fin - inicio).days > 365 * 5:
        return "❌ El rango no puede superar los 5 años."
    return None