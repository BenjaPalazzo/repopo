-- =============================================================================
-- Migración: refactor de prepaid_codes a modelo multi-uso
--
-- Cambios:
--   - Se eliminan redeemed_at, redeemed_by, redeemed_job (modelo uso único)
--   - Se agrega total_jobs  — cuántos trabajos otorga el código
--   - Se agrega used_jobs   — cuántos trabajos se consumieron hasta ahora
--   - Se agrega una tabla code_uses para auditoría de cada consumo
--   - La columna granted_tier representa la "capacidad" de los trabajos
--     (área, rango de tiempo, formatos disponibles), NO el tier de la cuenta
--
-- Aplicar con:
--   psql -U postgres -d sisar -f migrate_prepaid_codes.sql
-- =============================================================================

BEGIN;

-- -----------------------------------------------------------------------------
-- 1. Eliminar columnas del modelo de uso único
-- -----------------------------------------------------------------------------
ALTER TABLE prepaid_codes
    DROP COLUMN IF EXISTS redeemed_at,
    DROP COLUMN IF EXISTS redeemed_by,
    DROP COLUMN IF EXISTS redeemed_job;

-- -----------------------------------------------------------------------------
-- 2. Agregar columnas del modelo multi-uso
-- -----------------------------------------------------------------------------
ALTER TABLE prepaid_codes
    ADD COLUMN IF NOT EXISTS total_jobs  INTEGER NOT NULL DEFAULT 1
                                         CHECK (total_jobs > 0),
    ADD COLUMN IF NOT EXISTS used_jobs   INTEGER NOT NULL DEFAULT 0
                                         CHECK (used_jobs >= 0);

-- Garantizar que used_jobs nunca supere total_jobs
ALTER TABLE prepaid_codes
    ADD CONSTRAINT prepaid_codes_used_lte_total
        CHECK (used_jobs <= total_jobs);

-- -----------------------------------------------------------------------------
-- 3. Tabla de auditoría: un registro por cada consumo del código
-- -----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS code_uses (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    code        UUID        NOT NULL REFERENCES prepaid_codes(code),
    used_by     UUID        NOT NULL REFERENCES users(id),
    job_id      UUID        NOT NULL REFERENCES jobs(id),
    used_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_code_uses_code   ON code_uses(code);
CREATE INDEX IF NOT EXISTS idx_code_uses_job_id ON code_uses(job_id);

-- -----------------------------------------------------------------------------
-- 4. Comentarios descriptivos
-- -----------------------------------------------------------------------------
COMMENT ON COLUMN prepaid_codes.total_jobs IS
    'Número total de trabajos que otorga este código.';

COMMENT ON COLUMN prepaid_codes.used_jobs IS
    'Número de trabajos ya consumidos. Nunca supera total_jobs.';

COMMENT ON COLUMN prepaid_codes.granted_tier IS
    'Capacidad de los trabajos pagados con este código: define límites de '
    'área, rango temporal y formatos de resultado disponibles. '
    'No modifica el tier de cuenta del usuario.';

COMMENT ON TABLE code_uses IS
    'Registro de auditoría: un fila por cada trabajo submitido con un prepaid_code.';

COMMIT;
