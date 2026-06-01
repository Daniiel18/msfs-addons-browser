-- (v4.0.0 — P4) Métricas de motor por sample para el Performance modal.
--
-- 6 métricas × 4 engine slots = 24 columnas nuevas. MSFS expone hasta
-- 4 motores por avión; los slots sobrantes (ej. avión bimotor 3 y 4)
-- son 0.0 constantes y no se grafican.
--
-- Simvars MSFS / unidad SimConnect requested:
--   · TURB ENG N1:N                          → percent
--   · TURB ENG N2:N                          → percent
--   · GENERAL ENG EXHAUST GAS TEMPERATURE:N  → celsius
--   · ENG FUEL FLOW PPH:N                    → pounds per hour
--   · ENG OIL TEMPERATURE:N                  → celsius
--   · ENG OIL PRESSURE:N                     → psi
--
-- Decisión columnas separadas (vs JSON blob): el Performance modal
-- de v4 (P5) usa Recharts que consume arrays planos — leer 4 columnas
-- directas es más limpio que parse JSON cada query. Costo: ~24
-- columnas × ~360 samples/hora × ~70 bytes / sample = ~600 KB/hora
-- de vuelo. Despreciable contra los GBs de MSFS.
--
-- Todos los campos son nullable (REAL). Imports VAS-ACARS y vuelos
-- pre-v4 los dejan NULL — el modal Performance muestra "datos no
-- disponibles" para esas curvas o simplemente las omite.

-- N1 (% RPM del fan/low-pressure spool)
ALTER TABLE flight_log_track ADD COLUMN eng_n1_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n1_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n1_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n1_4 REAL;

-- N2 (% RPM del core/high-pressure spool)
ALTER TABLE flight_log_track ADD COLUMN eng_n2_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n2_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n2_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_n2_4 REAL;

-- EGT (Exhaust Gas Temperature, °C)
ALTER TABLE flight_log_track ADD COLUMN eng_egt_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_egt_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_egt_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_egt_4 REAL;

-- Fuel Flow (libras por hora — pph)
ALTER TABLE flight_log_track ADD COLUMN eng_ff_pph_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_ff_pph_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_ff_pph_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_ff_pph_4 REAL;

-- Oil Temperature (°C)
ALTER TABLE flight_log_track ADD COLUMN eng_oil_temp_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_temp_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_temp_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_temp_4 REAL;

-- Oil Pressure (psi)
ALTER TABLE flight_log_track ADD COLUMN eng_oil_press_1 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_press_2 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_press_3 REAL;
ALTER TABLE flight_log_track ADD COLUMN eng_oil_press_4 REAL;
