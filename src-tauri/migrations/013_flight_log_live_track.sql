-- ACARS-style: cada vuelo guarda su última posición conocida,
-- altitud y groundspeed por si la app se cierra a mitad de vuelo.
-- Al reabrirse, restauramos el vuelo en curso desde la última fila
-- abierta (`ended_at = NULL`) en lugar de crear una nueva entry
-- con origen "?" (que era el bug que reportó el usuario).
ALTER TABLE flight_log ADD COLUMN last_position_lat       REAL;
ALTER TABLE flight_log ADD COLUMN last_position_lon       REAL;
ALTER TABLE flight_log ADD COLUMN last_position_alt_ft    INTEGER;
ALTER TABLE flight_log ADD COLUMN last_position_gs_kt     INTEGER;
ALTER TABLE flight_log ADD COLUMN last_position_at        TEXT;
