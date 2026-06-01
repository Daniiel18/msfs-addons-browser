-- (v4.0.0 P7.6) Vínculo explícito flight_log → simbrief_flights.
--
-- Problema: cuando dos usuarios comparten cuenta SimBrief, el último
-- OFP descargado se atribuye automáticamente al primer vuelo nuevo
-- en el mismo aeropuerto de origen, sin importar quién lo planificó.
-- Resultado: el amigo que voló desde el mismo aeropuerto hereda el
-- flight_number / callsign / pax / cargo del OFP del otro usuario.
--
-- Solución: persistimos el ofp_id que cada vuelo consumió y filtramos
-- los OFPs "ya consumidos por un vuelo cerrado" en los lookups.
-- Adicionalmente, validamos cross-match por aircraft_icao para evitar
-- que un OFP planificado en A320 se aplique a un vuelo de B738.
ALTER TABLE flight_log ADD COLUMN simbrief_ofp_id TEXT;

CREATE INDEX IF NOT EXISTS idx_flight_log_simbrief_ofp_id
  ON flight_log(simbrief_ofp_id);
