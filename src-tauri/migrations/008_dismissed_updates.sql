-- Updates "marcadas como vistas" por el usuario.
--
-- Antes guardábamos esto en `localStorage` del frontend, pero el
-- usuario reportó dos problemas:
--   1. Al pulsar "Marcar todas como vistas" y luego "Recargar",
--      las descartadas no volvían a aparecer aunque seguían
--      pendientes de actualizar.
--   2. Cambiar de máquina perdía el estado.
--
-- Persistir en SQLite resuelve ambos. La regla nueva es estricta:
-- una entrada en esta tabla esconde la update, pero **el botón
-- "Recargar" del panel de notificaciones limpia toda la tabla** —
-- de modo que el usuario siempre puede volver a ver lo pendiente
-- pulsando recargar. Una update sólo desaparece definitivamente
-- cuando el usuario la actualiza (porque la versión instalada
-- alcanza a la del catálogo y `compute_available` ya no la emite).
CREATE TABLE IF NOT EXISTS dismissed_updates (
    folder_name      TEXT PRIMARY KEY,
    dismissed_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
