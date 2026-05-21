from telegram.ext import (
    ApplicationBuilder,
    CommandHandler,
    MessageHandler,
    ConversationHandler,
    filters,
    CallbackQueryHandler,
)

from config import (
    TOKEN,
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
)
from handlers import (
    # generales
    start,
    help_cmd,
    fin,
    cancelar,
    mensaje_generico,
    mis_trabajos,
    manejar_cancelacion_job,
    # flujo analizar
    cmd_analizar,
    recibir_ubicacion,
    recibir_ubicacion_texto,
    recibir_fecha_inicio,
    recibir_fecha_fin,
    manejar_seleccion_modo,
    manejar_confirmacion,
    # flujo ver_resultados (jobs propios)
    cmd_ver_resultados,
    manejar_seleccion_job_resultado,
    manejar_tipo_resultado,
    manejar_latlon_resultado,
    # flujo demos
    manejar_seleccion_demo,
    manejar_tipo_resultado_demo,
    manejar_latlon_demo,
    # flujo qr
    cmd_qr,
    recibir_codigo_texto,
    recibir_foto_qr,
    _pedir_resultado
)


def main():
    app = ApplicationBuilder().token(TOKEN).build()

    conv_handler = ConversationHandler(
        entry_points=[
            CommandHandler("analizar", cmd_analizar),
        ],
        states={
            ESPERANDO_UBICACION: [
                MessageHandler(filters.LOCATION, recibir_ubicacion),
                MessageHandler(filters.TEXT & ~filters.COMMAND, recibir_ubicacion_texto),
            ],
            ESPERANDO_FECHA_INICIO: [
                MessageHandler(filters.TEXT & ~filters.COMMAND, recibir_fecha_inicio),
            ],
            ESPERANDO_FECHA_FIN: [
                MessageHandler(filters.TEXT & ~filters.COMMAND, recibir_fecha_fin),
            ],
            ESPERANDO_MODO: [
                CallbackQueryHandler(manejar_seleccion_modo),
            ],
            ESPERANDO_CONFIRMACION: [
                CallbackQueryHandler(manejar_confirmacion),
            ],
        },
        fallbacks=[
            CommandHandler("cancelar", cancelar),
        ],
    )

    app.add_handler(CommandHandler("start",        start))
    app.add_handler(CommandHandler("help",         help_cmd))
    app.add_handler(CommandHandler("end",          fin))
    app.add_handler(CommandHandler("mis_trabajos", mis_trabajos))
    app.add_handler(conv_handler)

    # Flujo /ver_resultados
    ver_resultados_handler = ConversationHandler(
        entry_points=[CommandHandler("ver_resultados", cmd_ver_resultados)],
        states={
            # El usuario ve jobs propios + botón "Ver demos"
            ESPERANDO_JOB_SELECCION: [
                CallbackQueryHandler(manejar_seleccion_job_resultado, pattern=r"^resultado_job:|^seccion:demos$"),
            ],
            ESPERANDO_RESULTADO_TIPO: [
                CallbackQueryHandler(manejar_tipo_resultado, pattern=r"^res_tipo:"),
            ],
            ESPERANDO_LATLON: [
                MessageHandler(filters.TEXT & ~filters.COMMAND, manejar_latlon_resultado),
            ],
            # Flujo demos
            ESPERANDO_DEMO_SELECCION: [
                CallbackQueryHandler(manejar_seleccion_demo, pattern=r"^demo:"),
            ],
            ESPERANDO_DEMO_RESULTADO_TIPO: [
                CallbackQueryHandler(manejar_tipo_resultado_demo, pattern=r"^demo_tipo:"),
            ],
            ESPERANDO_DEMO_LATLON: [
                MessageHandler(filters.TEXT & ~filters.COMMAND, manejar_latlon_demo),
            ],
        },
        fallbacks=[CommandHandler("cancelar", cancelar)],
    )
    app.add_handler(ver_resultados_handler)

    # Flujo /qr — ingresar o escanear código de acceso
    qr_handler = ConversationHandler(
        entry_points=[CommandHandler("qr", cmd_qr)],
        states={
            ESPERANDO_CODIGO_TEXTO: [
                MessageHandler(filters.PHOTO, recibir_foto_qr),
                MessageHandler(filters.TEXT & ~filters.COMMAND, recibir_codigo_texto),
            ],
        },
        fallbacks=[CommandHandler("cancelar", cancelar)],
    )
    app.add_handler(qr_handler)

    # Callback para cancelar jobs desde /mis_trabajos
    app.add_handler(CallbackQueryHandler(manejar_cancelacion_job, pattern=r"^cancel:"))

    app.add_handler(MessageHandler(filters.TEXT & ~filters.COMMAND, mensaje_generico))

    logger.info("Bot iniciado ✅")
    app.run_polling()


if __name__ == "__main__":
    main()
