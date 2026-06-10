-- (v4.16.1 #5b) Persistencia del METAR real de salida/llegada por vuelo.
-- Hasta ahora el METAR venía del briefing LIVE del OFP de SimBrief (solo
-- visible si el OFP actual coincidía con el vuelo). Estas columnas guardan
-- el METAR capturado al FINALIZAR el vuelo, para consulta histórica en el
-- FlightBook (y se sincroniza a la nube). NULL en imports VAS / sin OFP.
ALTER TABLE flight_log ADD COLUMN metar_origin TEXT;
ALTER TABLE flight_log ADD COLUMN metar_dest TEXT;
