-- (v3.6.0 Phase H — Epic A/C/D)
-- Campos de Virtual Airline (VA): flight number, callsign, airline ICAO
-- + estado del vuelo (completed | partial | aborted).
--
-- · `flight_number` — ej. "DL115", "IB2741". Clave de negocio que el
--   ACARS expone (VAS-ACARS lo trae en `data.db` campo f10 del
--   `FlightInfo` protobuf). Para vuelos SimConnect, el watcher hará
--   cross-match contra el OFP de SimBrief activo por origen y lo
--   copiará al iniciar el vuelo.
--
-- · `callsign` — ej. "DAL115" (3-letter airline code + número). Lo
--   usa ATC; complementa `flight_number` (que usa el código IATA).
--   Mismo origen que `flight_number`.
--
-- · `airline_icao` — código de 3 letras (DAL, IBE, AAL). MÁS robusto
--   que agrupar por `aircraft_airline` (string libre que tiene
--   variantes "Delta"/"Delta Air Lines"). Permite agrupar vuelos por
--   aerolínea en el dashboard sin fuzzy matching.
--
-- · `status` — gate de calidad del vuelo. Sólo `completed` se muestra
--   en la UI por defecto. `partial` = importado pero falta OOOI
--   (block-out/touchdown/block-in incompleto en VAS data.db, vuelo
--   abortado mid-flight, ≤10 min total). `aborted` = el usuario
--   canceló explícitamente.
--
-- ## Estrategia de dedup
--
-- Antes: `(source, external_id)` dedup-eaba imports VAS-ACARS por
-- UUID del archivo. Pero el mismo `flight_number` reimportado con
-- otro UUID (re-grabación, día siguiente) entraba duplicado.
--
-- Ahora: index único parcial sobre `(airline_icao, flight_number,
-- date(started_at))` cuando los 3 son non-null. Si llega un INSERT
-- conflictivo el código de aplicación decide: el `completed` prevalece,
-- el `partial` se mantiene en DB pero con `status='partial'` (oculto en
-- la UI por defecto). El usuario respondió Q1=b: conservar partial,
-- ocultar.

ALTER TABLE flight_log ADD COLUMN flight_number TEXT;
ALTER TABLE flight_log ADD COLUMN callsign      TEXT;
ALTER TABLE flight_log ADD COLUMN airline_icao  TEXT;
ALTER TABLE flight_log ADD COLUMN status        TEXT NOT NULL DEFAULT 'completed';

-- Índice para búsquedas por flight_number (sidebar filter del usuario).
CREATE INDEX IF NOT EXISTS idx_flight_log_flight_number
    ON flight_log(flight_number)
    WHERE flight_number IS NOT NULL;

-- Índice para listar aerolíneas (GROUP BY airline_icao en el endpoint
-- list_airlines).
CREATE INDEX IF NOT EXISTS idx_flight_log_airline_icao
    ON flight_log(airline_icao)
    WHERE airline_icao IS NOT NULL;

-- Índice para hide/show del filtro de UI (WHERE status = 'completed').
CREATE INDEX IF NOT EXISTS idx_flight_log_status
    ON flight_log(status);

-- Dedup canónico (Q2=yes): mismo airline + flight_number + mismo día.
-- ALCANCE: sólo para status='completed' — permite N partials con la
-- misma triple (auditoria de imports fallidos) pero sólo 1 completed.
-- Eso se alinea con Q1=b del usuario: "conserva con status='partial'
-- y oculta en UI" — los partials viven en DB sin chocar entre sí.
CREATE UNIQUE INDEX IF NOT EXISTS uq_flight_log_va_dedup_completed
    ON flight_log(airline_icao, flight_number, date(started_at))
    WHERE airline_icao IS NOT NULL
      AND flight_number IS NOT NULL
      AND status = 'completed';
