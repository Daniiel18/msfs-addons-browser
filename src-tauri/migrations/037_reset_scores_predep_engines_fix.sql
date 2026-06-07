-- (v4.6.1) Reset de scores cacheados tras corregir el evaluador de
-- "motores apagados" en Pre-Departure.
--
-- Bug: la fase pre_departure INCLUYE el arranque de motores en el gate
-- (cold & dark → start → pushback), pero el evaluador tomaba el N1
-- MÁXIMO de la fase → siempre alto (motores ya arrancados) → la regla
-- marcaba ✗ aunque el usuario empezara correctamente en frío. Ahora usa
-- el N1 MÍNIMO (si en algún momento estuvieron apagados, fue un arranque
-- en frío → ✓).
--
-- El fix vive en el evaluador (rubric.rs), que re-lee el N1 por sample ya
-- guardado. Basta con limpiar el cache: al abrir la Flight Evaluation,
-- `score_get_report` recomputa con la lógica corregida — sin re-volar.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
