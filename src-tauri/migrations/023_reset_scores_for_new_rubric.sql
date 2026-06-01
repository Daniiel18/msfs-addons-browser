-- (v3.7.0 — Phase O) Reset de scores tras refactor del rubric.
--
-- El rubric pasó de 14 reglas / 9 phases a ~40 reglas / 13 phases
-- alineadas con la rúbrica VAS-ACARS. Los `rule_id` cambiaron
-- (ej. `smooth_landing` → `landing_smooth`, nuevos phases como
-- `takeoff_accel`, `final_approach`, etc.).
--
-- Las filas viejas en `flight_log_score_item` referencian rule_ids
-- que ya no existen — si las dejáramos, la UI mezclaría items
-- nuevos y antiguos al mostrar el breakdown.
--
-- La solución es **borrar todos los score items** y resetear las
-- columnas resumen en `flight_log`. La próxima vez que la UI pida
-- `score_get_report(flight_id)`, el comando detectará que no hay
-- score persistido y disparará `score_flight()` que recomputa con
-- el rubric nuevo.
--
-- Para vuelos pre-v3.7.0 (que sólo tienen lat/lon/alt/gs en su
-- track) muchas reglas nuevas devolverán `severity = "skipped"`
-- (sin penalizar). El total será proporcional a las reglas que SÍ
-- se pudieron evaluar. Es esperado y correcto.

DELETE FROM flight_log_score_item;

UPDATE flight_log
SET score_total = NULL,
    score_max   = NULL,
    score_grade = NULL;
