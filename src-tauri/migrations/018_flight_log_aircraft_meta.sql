-- (v3.5.0) Metadata extra de la aeronave capturada vía SimConnect
-- al disparar el OUT event (block-out, soltar el freno de mano con
-- motores corriendo).
--
-- El watcher añade una nueva data definition (DEFINE_ID_AIRCRAFT_META)
-- con simvars STRING256 — separada de la principal porque mezclar
-- FLOAT64 y STRING256 con `#[repr(C, packed(4))]` es propenso a errores
-- de alignment. El struct meta se suscribe una vez por sample (SECOND
-- period) y persistimos sólo al BlockOut + al cierre del vuelo.
--
-- · `aircraft_model`        — `ATC MODEL` (ej. "TT:ATCCOM.AC_MODEL_A20N.0.text").
--                             Es la referencia interna del addon — útil
--                             para agrupar vuelos por tipo aunque la
--                             livery cambie.
-- · `aircraft_airline`      — `ATC AIRLINE` (ej. "Iberia", "Delta").
--                             Configurable por el usuario al cargar la
--                             aircraft en el sim (Hangar → Customize).
-- · `aircraft_registration` — `ATC ID` (ej. "EC-MXY", "N12345"). La
--                             matrícula real visible en el cuerpo del
--                             avión. En MSFS 2024 `ATC TAIL NUMBER` está
--                             deprecada — `ATC ID` es la canónica.
--
-- Compatibilidad: añadimos columnas con DEFAULT NULL, sin migrar datos
-- históricos. Los vuelos viejos siguen sin esta info; los nuevos
-- empiezan a llenar.

ALTER TABLE flight_log ADD COLUMN aircraft_model TEXT;
ALTER TABLE flight_log ADD COLUMN aircraft_airline TEXT;
ALTER TABLE flight_log ADD COLUMN aircraft_registration TEXT;
