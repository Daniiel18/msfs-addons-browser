import type {
  AiracUpdateInfo,
  AvailableUpdate,
  CommunityPackage,
  FlightStatus,
  SimBriefFlight,
} from "./types";

/**
 * (v6.2.26 / R3) Lógica pura del chequeo pre-vuelo "¿Listo para volar?".
 *
 * Cruza el próximo vuelo (último OFP de SimBrief, o el origen/destino en
 * vivo del sim) con lo que el usuario tiene instalado y al día:
 *   · escenario de salida/llegada instalado, activo y sin update pendiente
 *   · AIRAC vigente
 *   · MSFS abierto
 *
 * Devuelve una lista de checks tipados; el componente resuelve i18n y
 * las acciones (activar paquete, buscar escenario, actualizar AIRAC…).
 * Mantener esto separado del render lo hace testeable y evita duplicar
 * la heurística ICAO↔paquete.
 */

export type CheckStatus = "ok" | "warn" | "info";

/** Acción sugerida por un check con problema. La ejecuta el modal. */
export type PreflightAction =
  | { kind: "search"; icao: string }
  | { kind: "enable"; folderName: string }
  | { kind: "airac" };

export interface PreflightCheck {
  id: string;
  status: CheckStatus;
  titleKey: string;
  titleArgs?: Record<string, string>;
  detailKey?: string;
  detailArgs?: Record<string, string>;
  action?: PreflightAction;
  /** Etiqueta i18n del botón de acción, si hay acción. */
  actionKey?: string;
}

export interface PreflightRoute {
  originIcao: string;
  originName: string | null;
  destinationIcao: string;
  destinationName: string | null;
  aircraftIcao: string | null;
  /** true si la ruta viene del OFP de SimBrief; false si del sim en vivo. */
  fromPlan: boolean;
}

export interface PreflightInput {
  /** OFP más reciente de SimBrief (o null). */
  plan: SimBriefFlight | null;
  /** Estado en vivo del sim/watcher (o null). */
  status: FlightStatus | null;
  packages: CommunityPackage[];
  airac: AiracUpdateInfo | null;
  updates: AvailableUpdate[];
}

export interface PreflightResult {
  route: PreflightRoute | null;
  checks: PreflightCheck[];
  /** Nº de checks en estado "warn" (accionables). */
  issues: number;
  /** true si no hay ningún warn — todo listo. */
  ready: boolean;
}

function pickRoute(
  plan: SimBriefFlight | null,
  status: FlightStatus | null,
): PreflightRoute | null {
  if (plan) {
    return {
      originIcao: plan.originIcao,
      originName: plan.originName,
      destinationIcao: plan.destinationIcao,
      destinationName: plan.destinationName,
      aircraftIcao: plan.aircraftIcao,
      fromPlan: true,
    };
  }
  if (status?.originIcao && status?.destinationIcao) {
    return {
      originIcao: status.originIcao,
      originName: status.originName,
      destinationIcao: status.destinationIcao,
      destinationName: status.destinationName,
      aircraftIcao: status.aircraftIcao,
      fromPlan: false,
    };
  }
  return null;
}

export function computePreflight(input: PreflightInput): PreflightResult {
  const { plan, status, packages, airac, updates } = input;
  const checks: PreflightCheck[] = [];
  const route = pickRoute(plan, status);

  // 1. Plan de vuelo.
  if (route) {
    checks.push({
      id: "plan",
      status: "ok",
      titleKey: "preflight.check.plan_ok",
      titleArgs: { orig: route.originIcao, dest: route.destinationIcao },
    });
  } else {
    checks.push({
      id: "plan",
      status: "info",
      titleKey: "preflight.check.plan_none",
      detailKey: "preflight.check.plan_none_hint",
    });
  }

  const findScenery = (icao: string): CommunityPackage | undefined => {
    const u = icao.toUpperCase();
    const matches = packages.filter(
      (p) => (p.icao ?? "").toUpperCase() === u && !p.isLibraryPack,
    );
    // Preferimos uno activo; si todos están desactivados, devolvemos el
    // primero para poder ofrecer "activar".
    return matches.find((p) => p.enabled !== false) ?? matches[0];
  };
  const findUpdate = (icao: string): AvailableUpdate | undefined =>
    updates.find((u) => u.icao.toUpperCase() === icao.toUpperCase());

  // 2 & 3. Escenario de salida y de llegada.
  const legs: Array<["dep" | "arr", string]> = route
    ? [
        ["dep", route.originIcao],
        ["arr", route.destinationIcao],
      ]
    : [];
  for (const [leg, icao] of legs) {
    const pkg = findScenery(icao);
    const upd = findUpdate(icao);
    if (!pkg) {
      checks.push({
        id: `${leg}_scn`,
        status: "info",
        titleKey: `preflight.check.${leg}_default`,
        titleArgs: { icao },
        detailKey: "preflight.check.default_hint",
        action: { kind: "search", icao },
        actionKey: "preflight.action.search",
      });
    } else if (pkg.enabled === false) {
      checks.push({
        id: `${leg}_scn`,
        status: "warn",
        titleKey: `preflight.check.${leg}_disabled`,
        titleArgs: { icao },
        action: { kind: "enable", folderName: pkg.folderName },
        actionKey: "preflight.action.enable",
      });
    } else if (upd) {
      checks.push({
        id: `${leg}_scn`,
        status: "warn",
        titleKey: `preflight.check.${leg}_update`,
        titleArgs: {
          icao,
          from: upd.installedVersion,
          to: upd.latestVersion,
        },
        action: { kind: "search", icao },
        actionKey: "preflight.action.update",
      });
    } else {
      checks.push({
        id: `${leg}_scn`,
        status: "ok",
        titleKey: `preflight.check.${leg}_ok`,
        titleArgs: { icao },
      });
    }
  }

  // 4. AIRAC.
  if (airac?.hasUpdate) {
    checks.push({
      id: "airac",
      status: "warn",
      titleKey: "preflight.check.airac_stale",
      titleArgs: { cycle: airac.latestCycle ?? "" },
      action: { kind: "airac" },
      actionKey: "preflight.action.airac",
    });
  } else if (airac) {
    checks.push({
      id: "airac",
      status: "ok",
      titleKey: "preflight.check.airac_ok",
      titleArgs: { cycle: airac.installedCycle ?? "" },
    });
  }

  // 5. MSFS abierto (informativo).
  if (status?.simRunning) {
    checks.push({
      id: "sim",
      status: "ok",
      titleKey: "preflight.check.sim_running",
    });
  }

  const issues = checks.filter((c) => c.status === "warn").length;
  return { route, checks, issues, ready: issues === 0 };
}
