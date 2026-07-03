-- (v6.2.22) Descarte de updates POR VERSIÓN.
--
-- Bug reportado: el usuario descartó el aviso de KJFK cuando el update era
-- v1.0.2; al salir v1.0.3 la campanita/dashboard/mapa NUNCA volvieron a
-- avisar, porque el descarte era por folder_name a secas y silenciaba
-- TODAS las versiones futuras de ese escenario.
--
-- Ahora se guarda la versión del catálogo que se descartó: el aviso solo
-- se oculta mientras el catálogo siga en ESA versión; cuando el developer
-- publica una más nueva, el aviso reaparece.
--
-- Se limpian las filas existentes (sin versión no se sabe qué se descartó):
-- los avisos pendientes reales reaparecen una vez y al descartarlos quedan
-- correctamente acotados a su versión.

ALTER TABLE dismissed_updates ADD COLUMN latest_version TEXT NOT NULL DEFAULT '';

DELETE FROM dismissed_updates;
