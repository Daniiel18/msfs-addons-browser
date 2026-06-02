-- (v3.21.0) Reset de scores cacheados tras el fix de fases del rubric.
--
-- Bug: los helpers del rubric consultaban los nombres VIEJOS de fase
-- ("climbing" / "approach" / "preflight" / "parking") mientras el
-- watcher ya persiste los nombres CANÓNICOS del scoring_phase
-- ("initial_climb" / "final_approach" / "pre_departure" / "arrived").
-- Resultado: esas fases salían "No data to evaluate" y el score era
-- erróneo. El scoring_phase POR SAMPLE ya está bien guardado en
-- flight_log_track, así que basta con re-evaluar.
--
-- Limpiamos el cache (items + columnas resumen). Al abrir la Flight
-- Evaluation, `score_get_report` ve el resumen en NULL y llama a
-- `score_flight`, que recomputa con el rubric corregido — sin pedirle
-- al usuario que vuelva a volar.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
