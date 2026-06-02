-- (v4.0.0 P7.5b) Discriminadores adicionales para multi-factor scoring de OFP.
--
-- aircraft_reg: matrícula que el piloto setea en SimBrief "Custom Tail"
-- (campo opcional). Cuando está presente y matchea con ATC ID del avión
-- en el sim, es el discriminador más fuerte para cuenta SimBrief
-- compartida — es el único campo per-pilot que SimBrief realmente expone.
--
-- plan_ramp_kg: fuel total al bloque planificado (enroute_burn + taxi +
-- reserve + alternate + extra). Comparable con FUEL TOTAL QUANTITY WEIGHT
-- capturado al OUT. Si el OFP planea 8500 kg y el sim reporta 8400 kg al
-- OUT (Δ < 10%), es el OFP del piloto que cargó el avión.
ALTER TABLE simbrief_flights ADD COLUMN aircraft_reg TEXT;
ALTER TABLE simbrief_flights ADD COLUMN plan_ramp_kg INTEGER;
