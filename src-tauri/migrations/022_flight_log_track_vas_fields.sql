-- (v3.7.0 — Phase O) VAS-ACARS aligned scoring fields.
--
-- Extiende `flight_log_track` con los simvars que captura el watcher
-- y que el rubric necesita para evaluar reglas estilo VAS:
--
--   · pitch / bank / g_force            — General (pitch ≤ 20, bank ≤ 30, g ≤ 2)
--   · vs_fpm                            — Landing (≤ -1000 fpm)
--   · ias_kt                            — Take-Off Accel (≤ 250) / Approach (≤ 200)
--   · flaps_pct / gear_down / spoilers  — phase-specific
--   · lights (nav/beacon/taxi/land/str) — phase-specific
--   · parking_brake                     — Pre-Departure / Arrived
--   · transponder_code                  — Taxi-Out (set IFR code)
--
-- Todos los campos son **opcionales** — para vuelos importados de
-- VAS-ACARS quedan NULL (el .bin no exporta lights/pitch/etc). Los
-- evaluators del rubric detectan que toda la columna sea NULL para
-- una phase y devuelven `severity=skipped` (puntos 0/0, sin penalizar).
--
-- Si en el futuro el formato ACARS expone alguno de estos (algunas
-- variantes nuevas de VAS-ACARS sí lo hacen), el importer puede
-- empezar a poblar los campos sin migrar de nuevo.

ALTER TABLE flight_log_track ADD COLUMN ias_kt           INTEGER;
ALTER TABLE flight_log_track ADD COLUMN vs_fpm           INTEGER;
ALTER TABLE flight_log_track ADD COLUMN pitch_deg        REAL;
ALTER TABLE flight_log_track ADD COLUMN bank_deg         REAL;
ALTER TABLE flight_log_track ADD COLUMN g_force          REAL;
ALTER TABLE flight_log_track ADD COLUMN flaps_pct        INTEGER;
ALTER TABLE flight_log_track ADD COLUMN gear_down        INTEGER;
ALTER TABLE flight_log_track ADD COLUMN spoilers_pct     INTEGER;
ALTER TABLE flight_log_track ADD COLUMN light_nav        INTEGER;
ALTER TABLE flight_log_track ADD COLUMN light_beacon     INTEGER;
ALTER TABLE flight_log_track ADD COLUMN light_taxi       INTEGER;
ALTER TABLE flight_log_track ADD COLUMN light_landing    INTEGER;
ALTER TABLE flight_log_track ADD COLUMN light_strobe     INTEGER;
ALTER TABLE flight_log_track ADD COLUMN parking_brake    INTEGER;
ALTER TABLE flight_log_track ADD COLUMN transponder_code INTEGER;
