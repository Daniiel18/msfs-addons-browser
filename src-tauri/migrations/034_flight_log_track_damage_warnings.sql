-- (v3.26.0 — P7.10) Analizador de daños / "vuelo forzado".
--
-- MSFS 2020 NO expone daño estructural / fallo de flaps-gear por
-- SimConnect (eso es de MSFS 2024). Lo que SÍ expone y usamos como
-- proxy del maltrato al avión:
--   · OVERSPEED WARNING  → se voló por encima de Vmo/Mmo.
--   · WARNING OIL PRESSURE → presión de aceite fuera de rango (daño motor).
--   · STALL WARNING (ya capturado, migración 031).
--   · G FORCE (ya capturado en g_force) → sobre-G estructural.
--
-- Capturamos los 2 warnings nuevos por track sample (0/1). El veredicto
-- LIMPIO / FORZADO / DAÑADO se computa en damage.rs leyendo estas
-- columnas + g_force a lo largo del vuelo. NULL en imports VAS / vuelos
-- viejos → el analizador lo marca "sin datos" sin penalizar.
ALTER TABLE flight_log_track ADD COLUMN overspeed_warning INTEGER;
ALTER TABLE flight_log_track ADD COLUMN oil_press_warning INTEGER;
