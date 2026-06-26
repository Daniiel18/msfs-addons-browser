-- (v6.1 Layer 2) Desgaste de neumático REAL leído del addon (iniBuilds A350
-- expone `INI_TIREx_WEAR` 0-100 vía LVar/MobiFlight). Lo guardamos como el
-- MÁXIMO alcanzado durante el vuelo (el peor neumático), igual que hacemos con
-- `max_brake_temp_c`. `NULL` en la inmensa mayoría de vuelos (addon sin este
-- dato o sin el módulo WASM de MobiFlight). El desgaste de neumáticos del panel
-- de mantenimiento lo usa como fuente REAL cuando existe (ver wear_for).
ALTER TABLE flight_log ADD COLUMN max_tire_wear_pct REAL;
