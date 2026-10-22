import { useEffect, useMemo } from "react";
import { computePreflight, pickTodayPlan, type PreflightResult } from "./preflight";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAiracUpdateStore } from "../stores/useAiracUpdateStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { usePreflightStore } from "../stores/usePreflightStore";
import { normalizeTechLogReg, useTechLogStore } from "../stores/useTechLogStore";
import { normalizeFerryReg, useFerryStore } from "../stores/useFerryStore";
import type { SimBriefFlight } from "./types";

/**
 * (v6.2.34) Hook compartido del pre-vuelo: cruza el OFP del DÍA con lo
 * instalado/al día + bypass del usuario. Lo usan el Dashboard (badge),
 * la campanita (aviso persistente) y el modal, para que todos coincidan.
 */
export function usePreflight(): {
  result: PreflightResult;
  plan: SimBriefFlight | null;
} {
  const flights = useSimBriefStore((s) => s.flights);
  const status = useFlightLogStore((s) => s.status);
  const packages = useCommunityStore((s) => s.packages);
  const updates = useCommunityStore((s) => s.updates);
  const airac = useAiracUpdateStore((s) => s.info);
  const gsxIcaos = useGsxLocalStore((s) => s.installedIcaos);
  const gsxUpdates = useGsxLocalStore((s) => s.updates);
  const bypass = usePreflightStore((s) => s.bypass);
  // (19/10) Despacho del avión del plan (TechLog / Tech Log de SimFleet).
  const dispatch = useTechLogStore((s) => s.dispatch);
  const epoch = useTechLogStore((s) => s.epoch);
  const managed = useTechLogStore((s) => s.managed);
  const todayPlan = useMemo(() => pickTodayPlan(flights), [flights]);
  const reg = todayPlan?.aircraftReg ?? status?.aircraftRegistration ?? null;
  useEffect(() => {
    if (reg) void useTechLogStore.getState().loadDispatch(reg);
  }, [reg, epoch, managed]);
  // (v8.3 · Etapa 6.1) Ubicación del avión del plan → aviso de traslado.
  // Recarga al pedir/cancelar un traslado y cuando se cierra un vuelo.
  const ferryInfo = useFerryStore((s) => s.info);
  const ferryEpoch = useFerryStore((s) => s.epoch);
  const flightsSig = useFlightLogStore(
    (s) => `${s.entries.length}|${s.entries[0]?.endedAt ?? ""}`,
  );
  const planOrigin = todayPlan?.originIcao ?? status?.originIcao ?? null;
  const planModel = todayPlan?.aircraftIcao ?? status?.aircraftIcao ?? null;
  useEffect(() => {
    if (reg) void useFerryStore.getState().load(reg, planOrigin, planModel);
  }, [reg, planOrigin, planModel, ferryEpoch, flightsSig]);

  return useMemo(() => {
    const plan = todayPlan;
    const bypassSet = new Set(
      Object.keys(bypass).filter((k) => bypass[k]),
    );
    // El Tech Log guarda la matrícula normalizada; se muestra como la trae el plan/sim.
    const mx = reg ? dispatch[normalizeTechLogReg(reg)] : undefined;
    const fi = reg ? ferryInfo[normalizeFerryReg(reg)] : undefined;
    const result = computePreflight({
      plan,
      status,
      packages,
      airac,
      updates,
      gsxInstalledIcaos: gsxIcaos,
      gsxUpdates,
      bypass: bypassSet,
      maintenance: mx && reg ? { ...mx, registration: reg.trim().toUpperCase() } : null,
      fleet:
        fi && reg
          ? { registration: reg.trim().toUpperCase(), model: planModel, ...fi }
          : null,
    });
    return { result, plan };
  }, [todayPlan, status, packages, updates, airac, gsxIcaos, gsxUpdates, bypass, reg, dispatch, ferryInfo, planModel]);
}
