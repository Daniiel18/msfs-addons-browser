-- (v4.12.0) Ruta planificada (navlog SimBrief) PEGADA a cada vuelo, como
-- JSON. Antes la ruta se resolvía contra el OFP en caché (frágil); ahora
-- se copia al flight_log al enlazar el OFP y viaja con el vuelo.
ALTER TABLE flight_log ADD COLUMN route_fixes TEXT;

-- Backfill: vuelos YA enlazados a un OFP que sigue en simbrief_flights.
UPDATE flight_log
SET route_fixes = (
    SELECT sf.route_fixes
    FROM simbrief_flights sf
    WHERE sf.ofp_id = flight_log.simbrief_ofp_id
)
WHERE simbrief_ofp_id IS NOT NULL
  AND route_fixes IS NULL;
