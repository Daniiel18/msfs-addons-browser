-- (v4.10.0) Plan de ruta (navlog de SimBrief) por OFP, como JSON.
-- Cada elemento: { ident, fixType, lat, lon, stage, isSidStar }.
-- Filas viejas quedan NULL → las SELECT usan COALESCE(route_fixes,'[]').
ALTER TABLE simbrief_flights ADD COLUMN route_fixes TEXT;
