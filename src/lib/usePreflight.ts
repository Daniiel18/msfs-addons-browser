import { useMemo } from "react";
import { computePreflight, pickTodayPlan, type PreflightResult } from "./preflight";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAiracUpdateStore } from "../stores/useAiracUpdateStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { usePreflightStore } from "../stores/usePreflightStore";
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

  return useMemo(() => {
    const plan = pickTodayPlan(flights);
    const bypassSet = new Set(
      Object.keys(bypass).filter((k) => bypass[k]),
    );
    const result = computePreflight({
      plan,
      status,
      packages,
      airac,
      updates,
      gsxInstalledIcaos: gsxIcaos,
      gsxUpdates,
      bypass: bypassSet,
    });
    return { result, plan };
  }, [flights, status, packages, updates, airac, gsxIcaos, gsxUpdates, bypass]);
}
