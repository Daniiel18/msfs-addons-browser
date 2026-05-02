-- Cache de búsquedas de perfiles GSX en flightsim.to.
--
-- Cada fila guarda el resultado JSON del scraper para un ICAO determinado
-- + el momento en que se consultó. El frontend pide `gsx_lookup(icao)` por
-- cada resultado de búsqueda; sin caché eso son N requests a flightsim.to
-- por cada vez que el usuario teclea algo. La idea es:
--
--   · TTL ~24h — los profiles GSX no se mueven seguido.
--   · `profiles_json` almacena la lista serializada (puede estar vacía
--     si no hay perfil para ese ICAO; eso también vale como negative cache
--     y evita reintentos en bucle).
--   · Una sola fila por ICAO, normalizado a mayúsculas — el endpoint de
--     flightsim.to es case-sensitive en algunos lados así que lo
--     normalizamos antes de guardar/leer.
CREATE TABLE IF NOT EXISTS gsx_lookups (
    icao            TEXT PRIMARY KEY,
    profiles_json   TEXT NOT NULL,
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_gsx_fetched_at ON gsx_lookups(fetched_at);
