-- (03/11) Etapa 8 — alternativos del plan (JSON: ["CYUL"]). Los OFP ya
-- guardados se rellenan al volver a pedirlos a SimBrief.
ALTER TABLE simbrief_flights ADD COLUMN alternate_icaos TEXT;
