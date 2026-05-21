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
    EJEMPLO_FECHA,
)
from utils import extraer_ubicacion_de_texto, parsear_fecha, validar_rango_fechas
from api import (
    registrar_usuario,
    enviar_job,
    obtener_job,
    listar_jobs,
    cancelar_job,
    obtener_velocidad,
    obtener_timeseries,
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
    Cuando termina, entrega los resultados o informa el error.
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
    """Descarga y envía los resultados al usuario según el modo elegido."""

    await context.bot.send_message(
        chat_id,
        "✅ <b>¡Procesamiento completado!</b> Preparando resultados…",
        parse_mode="HTML",
    )

    enviar_velocity   = modo in ("velocidad", "ambas")
    enviar_timeseries = modo in ("deformación", "ambas")

    if enviar_velocity:
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

    if enviar_timeseries:
        try:
            img_bytes = await obtener_timeseries(job_id, lat, lon, "png")
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
    await _get_or_register_uuid(update, context)

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
    """Punto de entrada de /ver_resultados — lista jobs completados."""
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

    if not completados:
        await update.message.reply_text(
            "📭 <b>No tenés trabajos completados aún.</b>\n\n"
            "Usá /analizar para iniciar un análisis.",
            parse_mode="HTML",
        )
        return ConversationHandler.END

    botones = [
        [InlineKeyboardButton(
            f"🗂 {j['id'][:8]}… — {j.get('created_at', '-')[:10]}",
            callback_data=f"resultado_job:{j['id']}"
        )]
        for j in completados
    ]

    await update.message.reply_text(
        "✅ <b>Trabajos completados</b>\nElegí uno para ver sus resultados:",
        parse_mode="HTML",
        reply_markup=InlineKeyboardMarkup(botones),
    )
    return ESPERANDO_JOB_SELECCION


async def manejar_seleccion_job_resultado(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """El usuario eligió un job — preguntamos qué tipo de resultado quiere."""
    query = update.callback_query
    await query.answer()

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
