-- (v4.32.0) Reset de scores cacheados tras dos arreglos:
--   1. arrived_parking_brake ahora evalúa la VENTANA FINAL del vuelo
--      con gs ≈ 0 (avión estacionado), no sólo la fase 'arrived'. El
--      piloto pone el freno al detenerse del todo, que puede ser tras
--      el último sample marcado 'arrived' (la máquina de fases oscila
--      si reposiciona). Antes daba "parking brake no aplicado" en
--      falso aunque el freno estuviera puesto.
--   2. El trigger de fin de vuelo pasó a N2<5% (de ENG COMBUSTION):
--      el watcher graba hasta que el motor casi se apaga, así captura
--      el parking brake del shutdown final.
-- El recompute ocurre al abrir la Flight Evaluation — sin re-volar.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
