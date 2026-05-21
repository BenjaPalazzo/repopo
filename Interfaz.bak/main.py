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
    # flujo ver_resultados
    cmd_ver_resultados,
    manejar_seleccion_job_resultado,
    manejar_tipo_resultado,
    manejar_latlon_resultado,
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
            ESPERANDO_JOB_SELECCION: [
                CallbackQueryHandler(manejar_seleccion_job_resultado, pattern=r"^resultado_job:"),
            ],
            ESPERANDO_RESULTADO_TIPO: [
                CallbackQueryHandler(manejar_tipo_resultado, pattern=r"^res_tipo:"),
            ],
            ESPERANDO_LATLON: [
                MessageHandler(filters.TEXT & ~filters.COMMAND, manejar_latlon_resultado),
            ],
        },
        fallbacks=[CommandHandler("cancelar", cancelar)],
    )
    app.add_handler(ver_resultados_handler)

    # Callback para cancelar jobs desde /mis_trabajos
    app.add_handler(CallbackQueryHandler(manejar_cancelacion_job, pattern=r"^cancel:"))

    app.add_handler(MessageHandler(filters.TEXT & ~filters.COMMAND, mensaje_generico))

    logger.info("Bot iniciado ✅")
    app.run_polling()


if __name__ == "__main__":
    main()
