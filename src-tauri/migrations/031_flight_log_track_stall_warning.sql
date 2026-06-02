-- (v3.21.0) STALL WARNING simvar (MSFS 2020) capturado por sample.
-- Alimenta la regla "Stall never indicated" del Flight Evaluation, que
-- antes quedaba "No data to evaluate" por no capturarse. 0/1.
ALTER TABLE flight_log_track ADD COLUMN stall_warning INTEGER;
