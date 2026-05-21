"""
handlers.py — Todos los handlers del bot ISAT.

Flujo principal (/analizar):
  1. Ubicación  (pin / texto / coords)
  2. Fecha inicio
  3. Fecha fin
  4. Selección de tarea  (deformación / velocidad / ambas)
  5. Confirmación → POST /jobs
  6. Polling de estado hasta completado o fallido
  7. Entrega de resultados (imagen velocity, timeseries)

Otros comandos:
  /start         → bienvenida + resumen de jobs
  /mis_trabajos  → lista completa de jobs
  /cancelar_job  → cancelar un job en curso
  /help          → ayuda
  /end           → cerrar sesión
  /cancelar      → abortar conversación activa
"""

import io
import asyncio
import httpx
from uuid import UUID

from telegram import (
    Update,
    ReplyKeyboardRemove,
    InlineKeyboardButton,
    InlineKeyboardMarkup,
    InputFile,
)
from telegram.ext import ContextTypes, ConversationHandler

from config import (
    logger,
    ESPERANDO_UBICACION,
    ESPERANDO_FECHA_INICIO,
    ESPERANDO_FECHA_FIN,
    ESPERANDO_MODO,
    ESPERANDO_CONFIRMACION,
    ESPERANDO_JOB_SELECCION,
    ESPERANDO_RESULTADO_TIPO,
    ESPERANDO_LATLON,
    ESPERANDO_CODIGO_TEXTO,
    ESPERANDO_DEMO_SELECCION,
    ESPERANDO_DEMO_RESULTADO_TIPO,
    ESPERANDO_DEMO_LATLON,
    EJEMPLO_FECHA,
)
from utils import extraer_ubicacion_de_texto, parsear_fecha, validar_rango_fechas
from api import (
    registrar_usuario,
    enviar_job,
    obtener_job,
    listar_jobs,
    cancelar_job,
    encolar_resultado,
    obtener_resultado,
    consultar_codigo,
    obtener_usuario_me,
    obtener_velocidad,
    obtener_timeseries,
    listar_demos,
    obtener_velocidad_demo,
    obtener_timeseries_demo,
)

# ──────────────────────────────────────────────
# Constantes de polling
# ──────────────────────────────────────────────

_POLL_INTERVAL   = 30    # segundos entre consultas de estado
_POLL_MAX_ROUNDS = 240   # máximo 240 × 30 s = 2 horas

# Estados terminales del servidor
_ESTADOS_FINALES  = {"completed", "failed", "cancelled"}
_ESTADOS_LEGIBLES = {
    "queued":              "⏳ En cola",
    "downloading":         "📡 Descargando imágenes",
    "isce_processing":     "⚙️ Procesando (ISCE2)",
    "mintpy_processing":   "📊 Analizando series de tiempo (MintPy)",
    "miaplpy_processing":  "📊 Analizando PS-InSAR (MiaplPy)",
    "results_generating":  "🗺️ Generando mapas de resultados",
    "completed":           "✅ Completado",
    "failed":              "❌ Fallido",
    "cancelled":           "🚫 Cancelado",
}

# ──────────────────────────────────────────────
# Helpers de auth internos
# ──────────────────────────────────────────────

async def _get_or_register_uuid(update: Update, context: ContextTypes.DEFAULT_TYPE) -> UUID | None:
    """
    Devuelve el UUID interno del usuario, registrándolo si es la primera vez.
    Guarda el UUID en context.user_data para no repetir la llamada.
    """
    if "user_uuid" in context.user_data:
        return context.user_data["user_uuid"]

    user = update.effective_user
    try:
        uuid = await registrar_usuario(user.id, user.first_name or user.username or "")
        context.user_data["user_uuid"] = uuid
        logger.info(f"Usuario registrado/recuperado: telegram_id={user.id} uuid={uuid}")
        return uuid
    except Exception as e:
        logger.error(f"Error al registrar usuario: {e}")
        await update.effective_message.reply_text(
            "❌ No se pudo conectar con el servidor. Intentá de nuevo más tarde."
        )
        return None


# ──────────────────────────────────────────────
# Helpers de presentación
# ──────────────────────────────────────────────

def _estado_legible(state: str, stage: str | None = None) -> str:
    base = _ESTADOS_LEGIBLES.get(state, state)
    if stage:
        return f"{base}\n   └ etapa: `{stage}`"
    return base


def _resumen_job(j: dict) -> str:
    estado = _estado_legible(j.get("state", "-"), j.get("stage"))
    creado = j.get("created_at", "-")[:10]
    return (
        f"🆔 <code>{j['id'][:8]}…</code>\n"
        f"📅 {creado}  |  {j.get('workflow', '-')}\n"
        f"Estado: {estado}\n"
    )


# ──────────────────────────────────────────────
# Polling de estado (tarea background)
# ──────────────────────────────────────────────

async def _poll_job(
    context: ContextTypes.DEFAULT_TYPE,
    chat_id: int,
    job_id: str,
    modo: str,
    lat: float,
    lon: float,
) -> None:
    """
    Consulta el estado del job cada _POLL_INTERVAL segundos.
    Cuando el job termina, encola los resultados y hace polling de esos también.
    """
    ultimo_estado = None

    for _ in range(_POLL_MAX_ROUNDS):
        await asyncio.sleep(_POLL_INTERVAL)

        try:
            job = await obtener_job(job_id)
        except Exception as e:
            logger.warning(f"Error al consultar job {job_id}: {e}")
            continue

        estado = job.get("state", "")
        stage  = job.get("stage")

        # Notificar cambios de estado
        if estado != ultimo_estado:
            texto = f"🔄 <b>Estado del job</b> <code>{job_id[:8]}…</code>\n{_estado_legible(estado, stage)}"
            await context.bot.send_message(chat_id, texto, parse_mode="HTML")
            ultimo_estado = estado

        if estado not in _ESTADOS_FINALES:
            continue

        # ── Terminal ──────────────────────────────
        if estado == "completed":
            await _entregar_resultados(context, chat_id, job_id, modo, lat, lon)
        elif estado == "failed":
            error = job.get("error") or "sin detalle"
            await context.bot.send_message(
                chat_id,
                f"❌ <b>El job falló.</b>\nDetalle: <code>{error}</code>\n\n"
                "Podés intentar de nuevo con /analizar.",
                parse_mode="HTML",
            )
        elif estado == "cancelled":
            await context.bot.send_message(
                chat_id,
                "🚫 El job fue cancelado.\n\nUsá /analizar para empezar uno nuevo.",
            )
        return

    # Timeout de polling
    await context.bot.send_message(
        chat_id,
        f"⚠️ No se recibió respuesta para el job <code>{job_id[:8]}…</code> en 2 horas.\n"
        "Podés consultar su estado con /mis_trabajos.",
        parse_mode="HTML",
    )


async def _entregar_resultados(
    context: ContextTypes.DEFAULT_TYPE,
    chat_id: int,
    job_id: str,
    modo: str,
    lat: float,
    lon: float,
) -> None:
    """Encola y entrega los resultados al usuario según el modo elegido."""

    await context.bot.send_message(
        chat_id,
        "✅ <b>¡Procesamiento completado!</b> Preparando resultados…",
        parse_mode="HTML",
    )

    enviar_velocity   = modo in ("velocidad", "ambas")
    enviar_timeseries = modo in ("deformación", "ambas")

    if enviar_velocity:
        try:
            img_bytes = await _pedir_resultado(job_id, "velocity", params=None, chat_id=chat_id, context=context)
            if img_bytes:
                await context.bot.send_photo(
                    chat_id,
                    photo=InputFile(io.BytesIO(img_bytes), filename="velocity.png"),
                    caption="🔀 *Mapa de velocidades de deformación*",
                    parse_mode="Markdown",
                )
        except Exception as e:
            logger.error(f"Error al obtener velocity para {job_id}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener el mapa de velocidades.")

    if enviar_timeseries:
        try:
            params = {"lat": lat, "lon": lon}
            img_bytes = await _pedir_resultado(job_id, "timeseries", params=params, chat_id=chat_id, context=context)
            if img_bytes:
                await context.bot.send_photo(
                    chat_id,
                    photo=InputFile(io.BytesIO(img_bytes), filename="timeseries.png"),
                    caption="🗺️ *Mapa de deformación del terreno*",
                    parse_mode="Markdown",
                )
        except Exception as e:
            logger.error(f"Error al obtener timeseries para {job_id}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener el mapa de deformación.")

    await context.bot.send_message(
        chat_id,
        "¿Necesitás algo más?\n📉 /analizar · 📂 /mis_trabajos",
    )


# Constantes de polling para result_requests
_RESULT_POLL_INTERVAL   = 15     # segundos
_RESULT_POLL_MAX_ROUNDS = 120    # 120 × 15 s = 30 minutos


async def _pedir_resultado(
    job_id: str,
    result_type: str,
    params: dict | None,
    chat_id: int,
    context: ContextTypes.DEFAULT_TYPE,
) -> bytes | None:
    """
    Encola un resultado y hace polling hasta que esté listo.
    Devuelve los bytes del archivo si todo salió bien, o None si falló.

    Para velocity con fast-path el servidor devuelve los bytes directamente
    en el POST; en ese caso no hay polling.
    """
    try:
        respuesta = await encolar_resultado(job_id, result_type, params)
    except Exception as e:
        logger.error(f"Error al encolar resultado {result_type} para {job_id}: {e}")
        return None

    # Fast-path: el servidor devolvió la imagen directamente (velocity ya generado).
    if isinstance(respuesta, bytes):
        return respuesta

    request_id = respuesta.get("result_request_id")
    if not request_id:
        logger.error(f"Respuesta inesperada al encolar {result_type}: {respuesta}")
        return None

    await context.bot.send_message(
        chat_id,
        f"⏳ Generando resultado <b>{result_type}</b>… te aviso cuando esté listo.",
        parse_mode="HTML",
    )

    for _ in range(_RESULT_POLL_MAX_ROUNDS):
        await asyncio.sleep(_RESULT_POLL_INTERVAL)

        try:
            res = await obtener_resultado(job_id, request_id)
        except Exception as e:
            logger.warning(f"Error al consultar resultado {request_id}: {e}")
            continue

        # Completado: servidor devolvió bytes.
        if isinstance(res, bytes):
            return res

        estado = res.get("state", "")

        if estado == "failed":
            error = res.get("error") or "sin detalle"
            logger.error(f"Resultado {request_id} falló: {error}")
            await context.bot.send_message(
                chat_id,
                f"⚠️ No se pudo generar el resultado <b>{result_type}</b>.\nDetalle: <code>{error}</code>",
                parse_mode="HTML",
            )
            return None

        # Sigue en queued/running — continuar polling.

    # Timeout
    await context.bot.send_message(
        chat_id,
        f"⚠️ Tiempo de espera agotado para el resultado <b>{result_type}</b>.\n"
        "Podés intentar de nuevo con /ver_resultados.",
        parse_mode="HTML",
    )
    return None


# ──────────────────────────────────────────────
# COMANDOS GENERALES
# ──────────────────────────────────────────────

async def start(update: Update, context: ContextTypes.DEFAULT_TYPE):
    user = update.effective_user
    context.user_data["user_id"] = user.id
    context.user_data["nombre"]  = user.first_name
    logger.info(f"Usuario: id={user.id} first_name={user.first_name}")

    uuid = await _get_or_register_uuid(update, context)

    resumen = ""
    if uuid:
        try:
            jobs = await listar_jobs(uuid)
            pendientes = [j for j in jobs if j.get("state") not in _ESTADOS_FINALES]
            completados = [j for j in jobs if j.get("state") == "completed"]
            fallidos    = [j for j in jobs if j.get("state") == "failed"]

            if not jobs:
                resumen = "📭 No tenés trabajos aún. ¡Podés pedir uno nuevo!\n\n"
            else:
                if pendientes:
                    resumen += f"⏳ <b>{len(pendientes)} trabajo(s) en proceso</b>\n"
                if completados:
                    resumen += f"✅ <b>{len(completados)} trabajo(s) completado(s)</b> — usá /mis_trabajos para verlos\n"
                if fallidos:
                    resumen += f"❌ <b>{len(fallidos)} trabajo(s) con error</b>\n"
                resumen += "\n"
        except Exception:
            resumen = ""

    await update.message.reply_text(
        f"👋 <b>Bienvenido, {user.first_name}!</b>\n\n"
        f"{resumen}"
        "🛰 <b>Bot satelital del ISAT</b>\n"
        "──────────────────────\n"
        "📉 /analizar       → Estudiar deformación de un terreno\n"
        "📂 /mis_trabajos   → Ver tus solicitudes\n"
        "🗺 /ver_resultados → Descargar resultados de un trabajo\n"
        "🎟 /qr             → Ingresar código de acceso\n"
        "ℹ️ /help           → Información\n"
        "🔚 /end            → Finalizar sesión\n"
        "──────────────────────",
        parse_mode="HTML",
    )


async def help_cmd(update: Update, context: ContextTypes.DEFAULT_TYPE):
    await update.message.reply_text(
        "<b>COMANDOS</b>\n\n"
        "📉 /analizar: Iniciá un análisis de deformación del terreno. El bot te pedirá "
        "ubicación, fechas y tipo de resultado (mapa de deformación, velocidades o ambos). "
        "El procesamiento puede demorar desde minutos hasta horas dependiendo del área y período.\n\n"
        "📂 /mis_trabajos: Muestra el estado de todas tus solicitudes.\n\n"
        "🗺 /ver_resultados: Descargá el mapa de velocidades o la serie temporal de un trabajo completado.\n\n"
        "🎟 /qr: Ingresá un código de acceso pegándolo como texto o enviando una foto del QR. "
        "El código habilita una cantidad de trabajos con la capacidad asociada.\n\n"
        "🔚 /end: Cerrá la sesión actual.\n\n"
        "Ante cualquier duda: isatcediac@gmail.com",
        parse_mode="HTML",
    )


async def mensaje_generico(update: Update, context: ContextTypes.DEFAULT_TYPE):
    await start(update, context)


# ──────────────────────────────────────────────
# VER TRABAJOS
# ──────────────────────────────────────────────

async def mis_trabajos(update: Update, context: ContextTypes.DEFAULT_TYPE):
    uuid = await _get_or_register_uuid(update, context)
    if not uuid:
        return

    await update.message.reply_text("📂 Buscando tus trabajos, aguardá un momento…")

    try:
        jobs = await listar_jobs(uuid)
    except Exception as e:
        logger.error(e)
        await update.message.reply_text("❌ No se pudo obtener tus trabajos. Intentá más tarde.")
        return

    if not jobs:
        await update.message.reply_text(
            "📭 <b>No tenés trabajos registrados aún.</b>\n\nUsá /analizar para hacer tu primera solicitud.",
            parse_mode="HTML",
        )
        return

    texto = "📂 <b>Tus trabajos</b>\n──────────────────────\n\n"
    for j in jobs:
        texto += _resumen_job(j) + "\n"

    # Botones de cancelación para jobs activos
    activos = [j for j in jobs if j.get("state") not in _ESTADOS_FINALES]
    markup = None
    if activos:
        botones = [
            [InlineKeyboardButton(
                f"🚫 Cancelar {j['id'][:8]}…",
                callback_data=f"cancel:{j['id']}"
            )]
            for j in activos
        ]
        markup = InlineKeyboardMarkup(botones)

    await update.message.reply_text(texto, parse_mode="HTML", reply_markup=markup)


async def manejar_cancelacion_job(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """Callback cuando el usuario toca 'Cancelar <job_id>' en /mis_trabajos."""
    query = update.callback_query
    await query.answer()

    job_id = query.data.split(":", 1)[1]
    try:
        ok = await cancelar_job(job_id)
        if ok:
            await query.edit_message_text(
                f"🚫 Job <code>{job_id[:8]}…</code> cancelado.",
                parse_mode="HTML",
            )
        else:
            await query.edit_message_text(
                f"⚠️ No se pudo cancelar el job <code>{job_id[:8]}…</code> "
                "(puede que ya esté en un estado terminal).",
                parse_mode="HTML",
            )
    except Exception as e:
        logger.error(e)
        await query.edit_message_text("❌ Error al cancelar el job.")


# ──────────────────────────────────────────────
# PASO 1 — Iniciar análisis
# ──────────────────────────────────────────────

async def cmd_analizar(update: Update, context: ContextTypes.DEFAULT_TYPE):
    user = update.effective_user
    context.user_data["user_id"] = user.id
    context.user_data["nombre"]  = user.first_name

    # Registrar usuario ya (para no tener que hacerlo al final)
    uuid = await _get_or_register_uuid(update, context)
    if not uuid:
        return ConversationHandler.END

    # Guard: verificar que el usuario tenga capacidad para analizar.
    # Los usuarios free/demo solo pueden proceder si tienen un código activo guardado.
    try:
        me = await obtener_usuario_me(uuid)
        tier = me.get("tier", "free")
    except Exception:
        tier = "free"

    tiene_codigo = bool(context.user_data.get("prepaid_code"))

    if tier == "free" and not tiene_codigo:
        await update.message.reply_text(
            "🔒 <b>Acceso limitado</b>\n\n"
            "Tu cuenta es <b>demo</b> y no tenés un código de acceso activo.\n\n"
            "Si tenés un código, usá /qr para ingresarlo y luego volvé a intentar.",
            parse_mode="HTML",
        )
        return ConversationHandler.END

    await update.message.reply_text(
        "📉 *Análisis de deformación del terreno*\n\n"
        "Enviá la ubicación de la zona de interés. Podés usar:\n"
        "• Un 📍 *pin de Telegram*\n"
        "• Un link de *Google Maps* o *OpenStreetMap*\n"
        "• Coordenadas directas: `-32.89, -68.84`\n\n"
        "Usá /cancelar para salir.",
        parse_mode="Markdown",
        reply_markup=ReplyKeyboardRemove(),
    )
    return ESPERANDO_UBICACION


# ──────────────────────────────────────────────
# PASO 2 — Recibir ubicación
# ──────────────────────────────────────────────

async def recibir_ubicacion(update: Update, context: ContextTypes.DEFAULT_TYPE):
    loc = update.message.location
    return await _guardar_ubicacion(update, context, loc.latitude, loc.longitude)


async def recibir_ubicacion_texto(update: Update, context: ContextTypes.DEFAULT_TYPE):
    resultado = extraer_ubicacion_de_texto(update.message.text)
    if resultado is None:
        await update.message.reply_text(
            "❌ No pude interpretar la ubicación. Podés enviar:\n"
            "• Un 📍 pin de Telegram\n"
            "• Un link de Google Maps o OpenStreetMap\n"
            "• Coordenadas: `-32.89, -68.84`",
            parse_mode="Markdown",
        )
        return ESPERANDO_UBICACION
    return await _guardar_ubicacion(update, context, *resultado)


async def _guardar_ubicacion(update, context, lat: float, lon: float):
    if not (-90 <= lat <= 90 and -180 <= lon <= 180):
        await update.message.reply_text("❌ Coordenadas fuera de rango. Intentá de nuevo.")
        return ESPERANDO_UBICACION

    context.user_data["lat"] = lat
    context.user_data["lon"] = lon

    await update.message.reply_text(
        f"📍 Ubicación recibida: `{lat:.5f}, {lon:.5f}`\n\n"
        f"Ahora ingresá la *fecha de inicio* en formato `YYYY-MM-DD`\n"
        f"Ejemplo: `{EJEMPLO_FECHA}`",
        parse_mode="Markdown",
    )
    return ESPERANDO_FECHA_INICIO


# ──────────────────────────────────────────────
# PASO 3 — Fecha inicio
# ──────────────────────────────────────────────

async def recibir_fecha_inicio(update: Update, context: ContextTypes.DEFAULT_TYPE):
    fecha = parsear_fecha(update.message.text)
    if fecha is None:
        await update.message.reply_text(
            f"❌ Formato inválido. Usá `YYYY-MM-DD`, por ejemplo: `{EJEMPLO_FECHA}`",
            parse_mode="Markdown",
        )
        return ESPERANDO_FECHA_INICIO

    context.user_data["fecha_inicio"]    = fecha.strftime("%Y-%m-%d")
    context.user_data["fecha_inicio_dt"] = fecha

    await update.message.reply_text(
        f"✅ Fecha de inicio: `{context.user_data['fecha_inicio']}`\n\n"
        f"Ahora ingresá la *fecha de fin* en formato `YYYY-MM-DD`\n"
        f"Ejemplo: `{EJEMPLO_FECHA}`",
        parse_mode="Markdown",
    )
    return ESPERANDO_FECHA_FIN


# ──────────────────────────────────────────────
# PASO 4 — Fecha fin
# ──────────────────────────────────────────────

async def recibir_fecha_fin(update: Update, context: ContextTypes.DEFAULT_TYPE):
    fecha = parsear_fecha(update.message.text)
    if fecha is None:
        await update.message.reply_text(
            f"❌ Formato inválido. Usá `YYYY-MM-DD`, por ejemplo: `{EJEMPLO_FECHA}`",
            parse_mode="Markdown",
        )
        return ESPERANDO_FECHA_FIN

    error = validar_rango_fechas(context.user_data["fecha_inicio_dt"], fecha)
    if error:
        await update.message.reply_text(error)
        return ESPERANDO_FECHA_FIN

    context.user_data["fecha_fin"] = fecha.strftime("%Y-%m-%d")
    return await _preguntar_tarea(update, context)


# ──────────────────────────────────────────────
# PASO 5 — Seleccionar tarea
# ──────────────────────────────────────────────

async def _preguntar_tarea(update: Update, context: ContextTypes.DEFAULT_TYPE):
    lat          = context.user_data["lat"]
    lon          = context.user_data["lon"]
    fecha_inicio = context.user_data["fecha_inicio"]
    fecha_fin    = context.user_data["fecha_fin"]
    delta        = 0.1   # ~11 km por lado → bbox razonable

    await update.message.reply_text(
        f"📦 *Resumen de la solicitud*\n\n"
        f"📍 Centro: `{lat:.5f}, {lon:.5f}`\n"
        f"📅 Período: `{fecha_inicio}` → `{fecha_fin}`\n"
        f"🗺 Bbox: N={lat+delta:.4f} S={lat-delta:.4f} E={lon+delta:.4f} W={lon-delta:.4f}\n\n"
        "¿Qué resultado querés obtener?",
        parse_mode="Markdown",
        reply_markup=InlineKeyboardMarkup([
            [InlineKeyboardButton("🗺️ Mapa de deformaciones",    callback_data="deformación")],
            [InlineKeyboardButton("🔀 Mapa de velocidades",       callback_data="velocidad")],
            [InlineKeyboardButton("🛰 Ambos mapas",               callback_data="ambas")],
        ]),
    )
    return ESPERANDO_MODO


async def manejar_seleccion_modo(update: Update, context: ContextTypes.DEFAULT_TYPE):
    query = update.callback_query
    await query.answer()

    modo = query.data
    context.user_data["modo"] = modo

    etiquetas = {
        "deformación": "🗺️ Mapa de deformaciones",
        "velocidad":   "🔀 Mapa de velocidades",
        "ambas":       "🛰 Ambos mapas",
    }

    # Determinar workflow según modo
    workflow_map = {
        "deformación": "sbas",
        "velocidad":   "sbas",
        "ambas":       "sbas",
    }
    context.user_data["workflow"] = workflow_map.get(modo, "sbas")

    await query.edit_message_text(
        f"✅ Seleccionado: *{etiquetas.get(modo, modo)}*\n\n"
        "¿Confirmás el envío del job?",
        parse_mode="Markdown",
        reply_markup=InlineKeyboardMarkup([
            [
                InlineKeyboardButton("✅ Confirmar", callback_data="confirmar"),
                InlineKeyboardButton("❌ Cancelar",  callback_data="cancelar_flujo"),
            ]
        ]),
    )
    return ESPERANDO_CONFIRMACION


# ──────────────────────────────────────────────
# PASO 6 — Confirmación y envío
# ──────────────────────────────────────────────

async def manejar_confirmacion(update: Update, context: ContextTypes.DEFAULT_TYPE):
    query = update.callback_query
    await query.answer()

    if query.data == "cancelar_flujo":
        await query.edit_message_text("❌ Operación cancelada.\n\nUsá /analizar para empezar de nuevo.")
        context.user_data.clear()
        return ConversationHandler.END

    # ── Enviar job ────────────────────────────
    await query.edit_message_text("⏳ Enviando job al servidor…")

    uuid         = context.user_data.get("user_uuid")
    lat          = context.user_data["lat"]
    lon          = context.user_data["lon"]
    fecha_inicio = context.user_data["fecha_inicio"]
    fecha_fin    = context.user_data["fecha_fin"]
    modo         = context.user_data.get("modo", "ambas")
    workflow     = context.user_data.get("workflow", "sbas")
    prepaid_code = context.user_data.get("prepaid_code")
    delta        = 0.1

    if uuid is None:
        await query.message.reply_text("❌ Error de sesión. Usá /analizar para reintentar.")
        return ConversationHandler.END

    try:
        resp = await enviar_job(
            user_uuid     = uuid,
            north         = lat + delta,
            south         = lat - delta,
            east          = lon + delta,
            west          = lon - delta,
            start         = fecha_inicio,
            end           = fecha_fin,
            workflow      = workflow,
            prepaid_code  = prepaid_code,
        )
    except httpx.HTTPStatusError as e:
        msg = str(e)
        # Mensajes de error amigables según código
        if "422" in msg or "no SAR images" in msg.lower():
            texto_error = (
                "⚠️ *No se encontraron imágenes satelitales* para esa zona y período.\n\n"
                "Probá con un área o rango de fechas diferente."
            )
        elif "403" in msg or "tier limit" in msg.lower():
            texto_error = "🚫 Límite de tu plan alcanzado. Contactanos para más info."
        else:
            texto_error = f"❌ Error al enviar el job:\n`{msg}`"

        await query.message.reply_text(texto_error, parse_mode="Markdown")
        return ConversationHandler.END

    except Exception as e:
        logger.error(f"Error inesperado al enviar job: {e}")
        await query.message.reply_text("❌ Error inesperado. Intentá de nuevo más tarde.")
        return ConversationHandler.END

    job_id   = resp["job_id"]
    path_num = resp.get("path", "?")
    chat_id  = update.effective_chat.id

    await query.message.reply_text(
        f"✅ <b>Job enviado correctamente.</b>\n\n"
        f"🆔 ID: <code>{job_id}</code>\n"
        f"🛰 Path Sentinel-1: <b>{path_num}</b>\n\n"
        f"Te voy a avisar cuando cambie el estado. El procesamiento puede demorar varias horas.\n"
        f"Podés ver el progreso en cualquier momento con /mis_trabajos.",
        parse_mode="HTML",
    )

    # Lanzar polling en background
    asyncio.create_task(
        _poll_job(context, chat_id, job_id, modo, lat, lon)
    )

    context.user_data.clear()
    return ConversationHandler.END


# ──────────────────────────────────────────────
# VER RESULTADOS
# ──────────────────────────────────────────────

async def cmd_ver_resultados(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """Punto de entrada de /ver_resultados — lista jobs completados y opción de demos."""
    uuid = await _get_or_register_uuid(update, context)
    if not uuid:
        return ConversationHandler.END

    await update.message.reply_text("📂 Buscando tus trabajos completados…")

    try:
        jobs = await listar_jobs(uuid)
    except Exception as e:
        logger.error(e)
        await update.message.reply_text("❌ No se pudo obtener tus trabajos. Intentá más tarde.")
        return ConversationHandler.END

    completados = [j for j in jobs if j.get("state") == "completed"]

    botones = []

    # Opción fija de demos (siempre visible).
    botones.append([InlineKeyboardButton("🎬 Ver demos", callback_data="seccion:demos")])

    # Jobs propios completados.
    for j in completados:
        botones.append([InlineKeyboardButton(
            f"🗂 {j['id'][:8]}… — {j.get('created_at', '-')[:10]}",
            callback_data=f"resultado_job:{j['id']}"
        )])

    texto = "✅ <b>Ver resultados</b>\nElegí una demo o uno de tus trabajos completados:"
    if not completados:
        texto = "✅ <b>Ver resultados</b>\nNo tenés trabajos completados aún, pero podés explorar las demos:"

    await update.message.reply_text(
        texto,
        parse_mode="HTML",
        reply_markup=InlineKeyboardMarkup(botones),
    )
    return ESPERANDO_JOB_SELECCION


async def manejar_seleccion_job_resultado(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario eligió un job o la sección de demos."""
    query = update.callback_query
    await query.answer()

    # ── Sección demos ────────────────────────────
    if query.data == "seccion:demos":
        await query.edit_message_text("🎬 Cargando demos disponibles…")
        try:
            demos = await listar_demos()
        except Exception as e:
            logger.error(e)
            await query.edit_message_text("❌ No se pudieron cargar las demos. Intentá más tarde.")
            return ConversationHandler.END

        if not demos:
            await query.edit_message_text(
                "📭 <b>No hay demos disponibles por el momento.</b>\n\n"
                "Usá /analizar para iniciar tu propio análisis.",
                parse_mode="HTML",
            )
            return ConversationHandler.END

        botones = [
            [InlineKeyboardButton(f"🗺 {d['name']}", callback_data=f"demo:{d['name']}")]
            for d in demos
        ]
        await query.edit_message_text(
            "🎬 <b>Demos disponibles</b>\nElegí una para ver sus resultados:",
            parse_mode="HTML",
            reply_markup=InlineKeyboardMarkup(botones),
        )
        return ESPERANDO_DEMO_SELECCION

    # ── Job propio ───────────────────────────────
    job_id = query.data.split(":", 1)[1]
    context.user_data["resultado_job_id"] = job_id

    await query.edit_message_text(
        f"🆔 Job: <code>{job_id[:8]}…</code>\n\n¿Qué resultado querés ver?",
        parse_mode="HTML",
        reply_markup=InlineKeyboardMarkup([
            [InlineKeyboardButton("🔀 Mapa de velocidades", callback_data="res_tipo:velocity")],
            [InlineKeyboardButton("📈 Serie temporal",      callback_data="res_tipo:timeseries")],
            [InlineKeyboardButton("🛰 Ambos",               callback_data="res_tipo:ambos")],
        ]),
    )
    return ESPERANDO_RESULTADO_TIPO


async def manejar_tipo_resultado(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario eligió el tipo — si necesita lat/lon lo pedimos, si no entregamos."""
    query = update.callback_query
    await query.answer()

    tipo = query.data.split(":", 1)[1]
    context.user_data["resultado_tipo"] = tipo

    if tipo in ("timeseries", "ambos"):
        await query.edit_message_text(
            "📍 Ingresá las coordenadas del punto que querés analizar.\n"
            "Formato: <code>lat, lon</code> — por ejemplo: <code>-32.89, -68.84</code>",
            parse_mode="HTML",
        )
        return ESPERANDO_LATLON
    else:
        # Solo velocity — no necesita coordenadas
        await query.edit_message_text("⏳ Descargando mapa de velocidades…")
        await _entregar_resultado_por_tipo(
            context=context,
            chat_id=update.effective_chat.id,
            job_id=context.user_data["resultado_job_id"],
            tipo="velocity",
            lat=None,
            lon=None,
        )
        context.user_data.clear()
        return ConversationHandler.END


async def manejar_latlon_resultado(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """Recibe lat/lon del usuario y entrega el resultado."""
    from utils import parsear_ubicacion_texto

    coords = parsear_ubicacion_texto(update.message.text)
    if coords is None:
        await update.message.reply_text(
            "❌ No pude interpretar las coordenadas. Usá el formato: <code>-32.89, -68.84</code>",
            parse_mode="HTML",
        )
        return ESPERANDO_LATLON

    lat, lon = coords
    tipo   = context.user_data.get("resultado_tipo", "timeseries")
    job_id = context.user_data["resultado_job_id"]

    await update.message.reply_text("⏳ Descargando resultados…")

    await _entregar_resultado_por_tipo(
        context=context,
        chat_id=update.effective_chat.id,
        job_id=job_id,
        tipo=tipo,
        lat=lat,
        lon=lon,
    )
    context.user_data.clear()
    return ConversationHandler.END


async def _entregar_resultado_por_tipo(
    context: ContextTypes.DEFAULT_TYPE,
    chat_id: int,
    job_id: str,
    tipo: str,
    lat: float | None,
    lon: float | None,
) -> None:
    """Descarga y envía velocity y/o timeseries según el tipo elegido."""

    if tipo in ("velocity", "ambos"):
        try:
            img_bytes = await obtener_velocidad(job_id)
            await context.bot.send_photo(
                chat_id,
                photo=InputFile(io.BytesIO(img_bytes), filename="velocity.png"),
                caption="🔀 *Mapa de velocidades de deformación*",
                parse_mode="Markdown",
            )
        except Exception as e:
            logger.error(f"Error al obtener velocity para {job_id}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener el mapa de velocidades.")

    if tipo in ("timeseries", "ambos") and lat is not None and lon is not None:
        try:
            img_bytes = await obtener_timeseries(job_id, lat, lon, "png")
            await context.bot.send_photo(
                chat_id,
                photo=InputFile(io.BytesIO(img_bytes), filename="timeseries.png"),
                caption="📈 *Serie temporal de deformación*",
                parse_mode="Markdown",
            )
        except Exception as e:
            logger.error(f"Error al obtener timeseries para {job_id}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener la serie temporal.")

    await context.bot.send_message(
        chat_id,
        "¿Necesitás algo más?\n📉 /analizar · 📂 /mis_trabajos · 🗺 /ver_resultados",
    )


# ──────────────────────────────────────────────
# CANJEAR CÓDIGO (/qr)
# ──────────────────────────────────────────────

def _formatear_info_codigo(info: dict) -> str:
    """Formatea la respuesta de GET /codes/:code para mostrar al usuario."""
    tier_labels = {
        "demo": "🔰 Demo",
        "free": "🆓 Free",
        "pro":  "⭐ Pro",
    }
    tier = tier_labels.get(info.get("capacity_tier", ""), info.get("capacity_tier", "-"))
    restantes = info.get("remaining_jobs", 0)
    total     = info.get("total_jobs", 0)
    usado     = info.get("used_jobs", 0)
    expira    = info.get("expires_at")
    valido    = info.get("is_valid", False)

    expira_str = expira[:10] if expira else "Sin vencimiento"

    estado = "✅ Activo" if valido else "❌ Inválido (agotado o vencido)"

    return (
        f"🎟 <b>Información del código</b>\n\n"
        f"📊 Capacidad: {tier}\n"
        f"🔢 Trabajos: {restantes}/{total} restantes ({usado} usados)\n"
        f"📅 Vence: {expira_str}\n"
        f"Estado: {estado}"
    )


async def cmd_qr(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """Punto de entrada de /qr — ofrece ingresar código por texto o foto."""
    codigo_actual = context.user_data.get("prepaid_code")

    texto_actual = ""
    if codigo_actual:
        texto_actual = f"\n\n📌 Código activo: <code>{codigo_actual}</code>"

    await update.message.reply_text(
        "🎟 <b>Ingresar código de acceso</b>\n\n"
        "Podés:\n"
        "• <b>Pegar el código</b> directamente (UUID, ej: <code>xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx</code>)\n"
        "• <b>Enviar una foto</b> del código QR\n\n"
        f"{texto_actual}\n"
        "Enviá el código o la foto. Usá /cancelar para salir.",
        parse_mode="HTML",
        reply_markup=ReplyKeyboardRemove(),
    )
    return ESPERANDO_CODIGO_TEXTO


async def recibir_codigo_texto(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario pegó el código como texto."""
    codigo = update.message.text.strip()
    return await _procesar_codigo(update, context, codigo)


async def recibir_foto_qr(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario envió una foto con el QR — intentamos decodificarlo."""
    try:
        from pyzbar.pyzbar import decode as qr_decode
        from PIL import Image
        import io as _io

        photo = update.message.photo[-1]  # la de mayor resolución
        file = await context.bot.get_file(photo.file_id)
        img_bytes = await file.download_as_bytearray()

        img = Image.open(_io.BytesIO(bytes(img_bytes)))
        decoded = qr_decode(img)

        if not decoded:
            await update.message.reply_text(
                "❌ No pude leer el QR de la imagen. Asegurate de que sea nítida "
                "o pegá el código como texto.",
            )
            return ESPERANDO_CODIGO_TEXTO

        codigo = decoded[0].data.decode("utf-8").strip()
        return await _procesar_codigo(update, context, codigo)

    except ImportError:
        await update.message.reply_text(
            "⚠️ El escaneo de QR no está disponible en este servidor.\n"
            "Por favor, pegá el código directamente como texto.",
        )
        return ESPERANDO_CODIGO_TEXTO
    except Exception as e:
        logger.error(f"Error al decodificar QR: {e}")
        await update.message.reply_text(
            "❌ Error al procesar la imagen. Intentá pegar el código como texto.",
        )
        return ESPERANDO_CODIGO_TEXTO


async def _procesar_codigo(
    update: Update,
    context: ContextTypes.DEFAULT_TYPE,
    codigo: str,
) -> int:
    """Consulta el servidor y muestra la info del código. Si es válido, lo guarda."""
    await update.effective_message.reply_text("🔍 Consultando código…")

    try:
        info = await consultar_codigo(codigo)
    except httpx.HTTPStatusError as e:
        if "404" in str(e):
            await update.effective_message.reply_text(
                "❌ Código no encontrado. Verificá que lo hayas ingresado correctamente.",
            )
        else:
            await update.effective_message.reply_text(
                f"❌ Error al consultar el código: {e}",
            )
        return ESPERANDO_CODIGO_TEXTO
    except Exception as e:
        logger.error(f"Error al consultar código: {e}")
        await update.effective_message.reply_text(
            "❌ No se pudo conectar con el servidor. Intentá más tarde.",
        )
        return ESPERANDO_CODIGO_TEXTO

    texto_info = _formatear_info_codigo(info)

    if info.get("is_valid"):
        # Guardar en contexto para usarlo al hacer submit
        context.user_data["prepaid_code"] = codigo
        await update.effective_message.reply_text(
            f"{texto_info}\n\n"
            "✅ <b>Código guardado.</b> Se usará en tu próximo análisis con /analizar.",
            parse_mode="HTML",
        )
    else:
        await update.effective_message.reply_text(
            f"{texto_info}\n\n"
            "⚠️ El código no está activo. No fue guardado.",
            parse_mode="HTML",
        )

    return ConversationHandler.END


# ──────────────────────────────────────────────
# DEMOS (dentro del flujo /ver_resultados)
# ──────────────────────────────────────────────

async def manejar_seleccion_demo(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario eligió una demo — preguntamos qué tipo de resultado quiere."""
    query = update.callback_query
    await query.answer()

    demo_name = query.data.split(":", 1)[1]
    context.user_data["demo_name"] = demo_name

    await query.edit_message_text(
        f"🗺 <b>Demo:</b> <code>{demo_name}</code>\n\n¿Qué resultado querés ver?",
        parse_mode="HTML",
        reply_markup=InlineKeyboardMarkup([
            [InlineKeyboardButton("🔀 Mapa de velocidades", callback_data="demo_tipo:velocity")],
            [InlineKeyboardButton("📈 Serie temporal",      callback_data="demo_tipo:timeseries")],
            [InlineKeyboardButton("🛰 Ambos",               callback_data="demo_tipo:ambos")],
        ]),
    )
    return ESPERANDO_DEMO_RESULTADO_TIPO


async def manejar_tipo_resultado_demo(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario eligió el tipo de resultado para la demo."""
    query = update.callback_query
    await query.answer()

    tipo = query.data.split(":", 1)[1]
    context.user_data["demo_tipo"] = tipo

    if tipo in ("timeseries", "ambos"):
        await query.edit_message_text(
            "📍 Ingresá las coordenadas del punto que querés analizar.\n"
            "Formato: <code>lat, lon</code> — por ejemplo: <code>-32.89, -68.84</code>",
            parse_mode="HTML",
        )
        return ESPERANDO_DEMO_LATLON
    else:
        # Solo velocity — no necesita coordenadas.
        await query.edit_message_text("⏳ Descargando mapa de velocidades de la demo…")
        await _entregar_resultado_demo(
            context=context,
            chat_id=update.effective_chat.id,
            demo_name=context.user_data["demo_name"],
            tipo="velocity",
            lat=None,
            lon=None,
        )
        context.user_data.clear()
        return ConversationHandler.END


async def manejar_latlon_demo(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """Recibe lat/lon del usuario y entrega el resultado de la demo."""
    from utils import parsear_ubicacion_texto

    coords = parsear_ubicacion_texto(update.message.text)
    if coords is None:
        await update.message.reply_text(
            "❌ No pude interpretar las coordenadas. Usá el formato: <code>-32.89, -68.84</code>",
            parse_mode="HTML",
        )
        return ESPERANDO_DEMO_LATLON

    lat, lon = coords
    tipo      = context.user_data.get("demo_tipo", "timeseries")
    demo_name = context.user_data["demo_name"]

    await update.message.reply_text("⏳ Descargando resultados de la demo…")

    await _entregar_resultado_demo(
        context=context,
        chat_id=update.effective_chat.id,
        demo_name=demo_name,
        tipo=tipo,
        lat=lat,
        lon=lon,
    )
    context.user_data.clear()
    return ConversationHandler.END


async def _entregar_resultado_demo(
    context: ContextTypes.DEFAULT_TYPE,
    chat_id: int,
    demo_name: str,
    tipo: str,
    lat: float | None,
    lon: float | None,
) -> None:
    """Descarga y envía velocity y/o timeseries de una demo según el tipo elegido."""

    if tipo in ("velocity", "ambos"):
        try:
            img_bytes = await obtener_velocidad_demo(demo_name)
            await context.bot.send_photo(
                chat_id,
                photo=InputFile(io.BytesIO(img_bytes), filename="velocity.png"),
                caption=f"🔀 *Mapa de velocidades — Demo {demo_name}*",
                parse_mode="Markdown",
            )
        except Exception as e:
            logger.error(f"Error al obtener velocity de demo {demo_name}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener el mapa de velocidades.")

    if tipo in ("timeseries", "ambos") and lat is not None and lon is not None:
        try:
            img_bytes = await obtener_timeseries_demo(demo_name, lat, lon, "png")
            await context.bot.send_photo(
                chat_id,
                photo=InputFile(io.BytesIO(img_bytes), filename="timeseries.png"),
                caption=f"📈 *Serie temporal — Demo {demo_name}*",
                parse_mode="Markdown",
            )
        except Exception as e:
            logger.error(f"Error al obtener timeseries de demo {demo_name}: {e}")
            await context.bot.send_message(chat_id, "⚠️ No se pudo obtener la serie temporal.")

    await context.bot.send_message(
        chat_id,
        "¿Necesitás algo más?\n📉 /analizar · 📂 /mis_trabajos · 🗺 /ver_resultados",
    )


# ──────────────────────────────────────────────
# FIN / CANCELAR
# ──────────────────────────────────────────────

async def fin(update: Update, context: ContextTypes.DEFAULT_TYPE):
    nombre = context.user_data.get("nombre", "")
    context.user_data.clear()
    await update.message.reply_text(
        f"👋 <b>¡Hasta luego{', ' + nombre if nombre else ''}!</b>\n\n"
        "Si necesitás algo más, escribí /start.",
        parse_mode="HTML",
    )


async def cancelar(update: Update, context: ContextTypes.DEFAULT_TYPE):
    context.user_data.clear()
    await update.message.reply_text(
        "❌ Operación cancelada.\n\nUsá /analizar para empezar de nuevo.",
    )
    return ConversationHandler.END
