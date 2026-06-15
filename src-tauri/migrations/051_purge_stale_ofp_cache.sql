-- (v5.0.0 M3) Purga de OFPs colgados en la caché de SimBrief.
-- Con cuentas compartidas, el endpoint `fetchlast` devolvía OFPs de
-- varios pilotos y la tabla acumuló decenas de planes viejos que
-- saturaban el modal de confirmación al apagar motores. Empezamos
-- limpio: vaciamos la caché de PLANES (no toca el flight_log real, que
-- guarda los vuelos volados con su `simbrief_ofp_id` ya enlazado). La
-- app vuelve a poblarla con el poll pasivo de 30s durante la planificación.
DELETE FROM simbrief_flights;
