-- 061 (v6.2.67) Limpia el caché de lookups GSX. Las filas de `gsx_lookups` se
-- grabaron con el JSON viejo de GsxProfile (sin `downloads`/`rating`, agregados
-- en v6.2.64), así que el modal de oferta no podía marcar "Más popular" (los
-- downloads venían null desde el caché). Vaciando la tabla, el próximo lookup
-- re-consulta flightsim.to y guarda el JSON completo.
DELETE FROM gsx_lookups;
