import logging
import os
from dotenv import load_dotenv

load_dotenv()

# ──────────────────────────────────────────────
# TOKEN
# ──────────────────────────────────────────────
TOKEN        = os.getenv("TOKEN")
SERVER_TOKEN = os.getenv("SERVER_TOKEN", "")   # Bearer token para el servidor SISAR

# ──────────────────────────────────────────────
# ESTADOS del ConversationHandler
# ──────────────────────────────────────────────
ESPERANDO_UBICACION    = 1
ESPERANDO_FECHA_INICIO = 2
ESPERANDO_FECHA_FIN    = 3
ESPERANDO_MODO         = 4
ESPERANDO_CONFIRMACION = 5   # confirmación antes de enviar el job

# Estados para /ver_resultados
ESPERANDO_JOB_SELECCION  = 6
ESPERANDO_RESULTADO_TIPO = 7
ESPERANDO_LATLON         = 8

# Estados para /demos (dentro de /ver_resultados)
ESPERANDO_DEMO_SELECCION      = 11
ESPERANDO_DEMO_RESULTADO_TIPO = 12
ESPERANDO_DEMO_LATLON         = 13

# Estados para /qr
ESPERANDO_CODIGO_TEXTO = 9
ESPERANDO_QR_FOTO      = 10

# ──────────────────────────────────────────────
# FORMATO DE FECHA ESPERADO
# ──────────────────────────────────────────────
FORMATO_FECHA = "%Y-%m-%d"
EJEMPLO_FECHA = "2024-01-15"

# ──────────────────────────────────────────────
# LOGGING
# ──────────────────────────────────────────────
logging.basicConfig(
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    level=logging.INFO,
)
logger = logging.getLogger(__name__)
