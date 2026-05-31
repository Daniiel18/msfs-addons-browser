-- (v3.6.0 Phase H — Epic E)
-- Sistema de puntuación de vuelo estilo Virtual Airline.
--
-- El modelo se compone de TRES piezas:
--
-- 1. **Columnas resumen** en `flight_log` — `score_total`, `score_max`,
--    `score_grade` ('A'..'F'). Permiten leer el score de un vuelo sin
--    join (lo usa el badge en el sidebar, el header del checklist).
--
-- 2. **`flight_log_score_item`** — breakdown rule-by-rule. Cada vuelo
--    tiene ~20-40 items: una fila por regla evaluada (ej.
--    "no_overspeed_below_10k", "smooth_landing", "gear_down_by_1000agl").
--    El widget Checklist agrupa estas filas por `phase` y suma los
--    puntos para mostrar el "590/590" por fase.
--
-- 3. **`flight_log_phase`** — registro temporal de cada transición de
--    fase del vuelo (preflight → engine_running → pushback → ...).
--    El watcher de SimConnect lo escribe en tiempo real; el importer de
--    VAS-ACARS lo deriva post-hoc del `flight_log_track` aplicando
--    `derive_phase_label` sample-a-sample. Los evaluators consultan
--    esta tabla para acotar la ventana temporal de cada regla
--    (ej. "no overspeed durante climb" lee `flight_log_track` filtrado
--    por la ventana `[climb.entered_at, climb.exited_at]`).
--
-- ## Diseño de keys / dedup
--
-- · `flight_log_score_item.(flight_id, phase, rule_id)` UNIQUE — para
--   permitir re-scoring idempotente (UPSERT en lugar de INSERT
--   duplicado al recalcular).
--
-- · `flight_log_phase.(flight_id, phase, entered_at)` UNIQUE — un
--   vuelo puede tener múltiples "taxi_out" rows si el watcher detecta
--   stop&go en taxi, pero el `entered_at` desambigua. Permite re-importar
--   sin acumular fantasmas.
--
-- · ON DELETE CASCADE — borrar un vuelo limpia automáticamente sus
--   score items y phase transitions (sin huérfanos).
--
-- ## Severity vs. passed
--
-- `passed` es booleano: cumplió o no la regla.
-- `severity` califica el incumplimiento (cuando passed=0):
--   - `info`  — observación sin penalización
--   - `warn`  — penalización parcial (resta pero no es grave)
--   - `fail`  — pérdida total de los puntos de la regla
-- Permite mostrar la regla con un color (verde/amber/red) en la UI.
--
-- ## evidence (TEXT/JSON)
--
-- Cada item guarda evidencia humana legible — los números observados
-- que dispararon el pass/fail. Ej. `{"observed_alt":9200,"observed_ias":263,"limit":250}`.
-- El widget despliega la evidencia al expandir la regla.

-- ============================================================================
-- 1. Columnas resumen en flight_log.
-- ============================================================================
ALTER TABLE flight_log ADD COLUMN score_total INTEGER;
ALTER TABLE flight_log ADD COLUMN score_max   INTEGER;
ALTER TABLE flight_log ADD COLUMN score_grade TEXT;

-- ============================================================================
-- 2. Breakdown rule-by-rule.
-- ============================================================================
CREATE TABLE IF NOT EXISTS flight_log_score_item (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    flight_id       INTEGER NOT NULL REFERENCES flight_log(id) ON DELETE CASCADE,
    phase           TEXT    NOT NULL,
        -- 'general' | 'pre_departure' | 'pushback' | 'taxi_out' |
        -- 'takeoff' | 'initial_climb' | 'climb' | 'cruise' | 'descent' |
        -- 'initial_approach' | 'final_approach' | 'landing' | 'taxi_in' |
        -- 'arrived'
    rule_id         TEXT    NOT NULL,
    label           TEXT    NOT NULL,
    points_earned   INTEGER NOT NULL,
    points_max      INTEGER NOT NULL,
    passed          INTEGER NOT NULL,   -- 0/1
    severity        TEXT,                -- 'info'|'warn'|'fail'
    evidence        TEXT,                -- JSON blob
    ts              TEXT
);

CREATE INDEX IF NOT EXISTS idx_score_item_flight
    ON flight_log_score_item(flight_id);
CREATE INDEX IF NOT EXISTS idx_score_item_flight_phase
    ON flight_log_score_item(flight_id, phase);
CREATE UNIQUE INDEX IF NOT EXISTS uq_score_item_flight_rule
    ON flight_log_score_item(flight_id, phase, rule_id);

-- ============================================================================
-- 3. Phases del vuelo — temporal log de transiciones.
-- ============================================================================
CREATE TABLE IF NOT EXISTS flight_log_phase (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    flight_id    INTEGER NOT NULL REFERENCES flight_log(id) ON DELETE CASCADE,
    phase        TEXT    NOT NULL,
    entered_at   TEXT    NOT NULL,   -- ISO-8601 UTC
    exited_at    TEXT                -- NULL mientras la phase está activa
);

CREATE INDEX IF NOT EXISTS idx_phase_flight
    ON flight_log_phase(flight_id);
CREATE INDEX IF NOT EXISTS idx_phase_flight_entered
    ON flight_log_phase(flight_id, entered_at);
CREATE UNIQUE INDEX IF NOT EXISTS uq_phase_flight_phase_entered
    ON flight_log_phase(flight_id, phase, entered_at);
