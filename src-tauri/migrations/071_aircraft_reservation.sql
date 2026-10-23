-- (v8.3 · Etapa 6.2d) Reservas: un avión del usuario que está volando de
-- verdad (OpenSky en vivo) no se puede trasladar, pero se puede reservar:
-- al aterrizar queda para el usuario y sus vuelos reales posteriores dejan
-- de moverlo. Se libera al volarlo, al cancelarla o a los 7 días.
-- status: active | released | cancelled | expired.
-- `landed_*`: aterrizaje detectado en vivo (antes de que llegue el lote del
-- día de OpenSky con el aeropuerto de llegada).
CREATE TABLE IF NOT EXISTS aircraft_reservation (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    registration  TEXT    NOT NULL,      -- normalizada (A-Z0-9)
    icao24        TEXT,
    reserved_at   TEXT    NOT NULL,
    expires_at    TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'active',
    ended_at      TEXT,
    airborne      INTEGER NOT NULL DEFAULT 0,  -- estaba en el aire al reservar
    landed_icao   TEXT,
    landed_at     TEXT
);

CREATE INDEX IF NOT EXISTS idx_aircraft_reservation_reg ON aircraft_reservation (registration, status);
