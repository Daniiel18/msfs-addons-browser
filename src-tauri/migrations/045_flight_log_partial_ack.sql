-- (v4.23.0) Vuelos INTERRUMPIDOS (apagon de PC / cierre abrupto del sim).
-- `partial_ack = 0` marca un vuelo cerrado como 'partial' que el usuario
-- aun NO revisó — al abrir la app sale un modal con el mapa del track y
-- las metricas preguntando si mantenerlo (pasa a completed) o eliminarlo.
-- DEFAULT 1: las filas existentes (incluidos partial viejos de VAS) no
-- generan el modal retroactivamente.
ALTER TABLE flight_log ADD COLUMN partial_ack INTEGER NOT NULL DEFAULT 1;
