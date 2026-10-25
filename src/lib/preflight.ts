import type {
  AiracUpdateInfo,
  AircraftLocation,
  AvailableUpdate,
  CommunityPackage,
  FerryQuote,
  FlightStatus,
  GsxProfileUpdate,
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
  | { kind: "gsx"; icao: string }
  | { kind: "gsx-update"; icao: string; link: string }
  | { kind: "airac" }
  | { kind: "hangar"; registration: string }
  // (v8.3 · Etapa 6.1) Traslado del avión al origen del plan.
  | { kind: "ferry"; registration: string; toIcao: string; model: string | null; cost: string; duration: string }
  | { kind: "ferry-cancel"; id: number; toIcao: string }
  // (6.2d) Reservar el avión real que vuela ahora / cancelar la reserva.
  | { kind: "reserve"; registration: string }
  | { kind: "reserve-cancel"; id: number };

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
  /** ICAO al que aplica el bypass "usar escenario por defecto" (sólo en
   *  checks de escenario faltante/desactivado/update). Cuando está, el
   *  modal ofrece un check para omitir este punto. */
  bypassIcao?: string;
  /** (24/10) Hora (ISO) de una cuenta atrás tras el título (traslado en curso). */
  countdownTo?: string;
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
  /** ICAOs con perfil GSX instalado. Si está vacío, se asume que el
   *  usuario no usa GSX y se OMITEN los checks de GSX. */
  gsxInstalledIcaos?: Set<string>;
  /** (v6.2.44) Perfiles GSX instalados CON update disponible (ICAO
   *  upper → info con el link del perfil en flightsim.to). */
  gsxUpdates?: Map<string, GsxProfileUpdate>;
  /** ICAOs marcados como "usaré el escenario por defecto" — sus checks
   *  de escenario se dan por resueltos (bypass). */
  bypass?: Set<string>;
  /** (19/10) Despacho del avión del plan (TechLog o Tech Log de SimFleet). */
  maintenance?: PreflightMaintenance | null;
  /** (v8.3) Dónde está el avión del plan y cuánto costaría trasladarlo. */
  fleet?: PreflightFleet | null;
}

export interface PreflightFleet {
  registration: string;
  model: string | null;
  location: AircraftLocation;
  /** Traslado desde donde está hasta `origin` (null si no hace falta o no
   *  se pudo calcular). */
  quote: FerryQuote | null;
  origin: string | null;
}

/** "1 h 05 min" / "45 min". */
export function fmtFerryDuration(minutes: number): string {
  const m = Math.max(0, Math.round(minutes));
  const h = Math.floor(m / 60);
  const rest = m % 60;
  return h > 0 ? `${h} h ${String(rest).padStart(2, "0")} min` : `${rest} min`;
}

/** "$13,120" (USD sin decimales, como Finanzas). */
export function fmtFerryCost(usd: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0,
  }).format(usd);
}

/** "23 oct, 14:30" (fecha corta + hora local). */
function fmtDayClock(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime())
    ? ""
    : d.toLocaleString([], { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

export interface PreflightMaintenance {
  registration: string;
  appName: string;
  /** "GO" | "GO_MEL" | "NO_GO" */
  status: string;
  reasons: string[];
}

/** (v6.2.37 B3) ¿Es `iso` del "día" del vuelo? Robusto ante zonas
 *  horarias / medianoche: acepta si es el MISMO día local O si cae en
 *  las últimas 12 h (y no más de 6 h en el futuro). Un OFP de hace días
 *  queda fuera; uno de "hoy" generado justo antes/después de medianoche
 *  no se pierde por la TZ. */
function isRecentPlan(iso: string | null | undefined): boolean {
  if (!iso) return false;
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return false;
  const d = new Date(t);
  const now = new Date();
  const sameLocalDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameLocalDay) return true;
  const deltaMs = now.getTime() - t;
  return deltaMs >= -6 * 3600_000 && deltaMs < 12 * 3600_000;
}

/** OFP del DÍA más reciente (por generación/fetch). El pre-vuelo sólo
 *  considera el plan del día — no uno de hace días. `null` si no hay. */
export function pickTodayPlan(flights: SimBriefFlight[]): SimBriefFlight | null {
  const today = flights.filter(
    (f) => isRecentPlan(f.generatedAt) || isRecentPlan(f.fetchedAt),
  );
  if (today.length === 0) return null;
  return [...today].sort((a, b) =>
    (b.generatedAt ?? b.fetchedAt ?? "").localeCompare(
      a.generatedAt ?? a.fetchedAt ?? "",
    ),
  )[0];
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
  const bypass = input.bypass ?? new Set<string>();
  const gsxIcaos = input.gsxInstalledIcaos ?? new Set<string>();
  const gsxEnabled = gsxIcaos.size > 0;
  const gsxUpdatesMap =
    input.gsxUpdates ?? new Map<string, GsxProfileUpdate>();
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

  // 1b. (19/10) Mantenimiento del avión: con NO-GO no se despacha.
  const mx = input.maintenance;
  if (mx) {
    const detailArgs = { reason: mx.reasons[0] ?? "", app: mx.appName };
    if (mx.status === "NO_GO") {
      checks.push({
        id: "mx",
        status: "warn",
        titleKey: "preflight.check.mx_nogo",
        titleArgs: { reg: mx.registration, n: String(mx.reasons.length) },
        detailKey: "preflight.check.mx_detail",
        detailArgs,
        action: { kind: "hangar", registration: mx.registration },
        actionKey: "preflight.action.hangar",
      });
    } else if (mx.status === "GO_MEL") {
      checks.push({
        id: "mx",
        status: "info",
        titleKey: "preflight.check.mx_mel",
        titleArgs: { reg: mx.registration },
        detailKey: "preflight.check.mx_detail",
        detailArgs,
      });
    } else {
      checks.push({
        id: "mx",
        status: "ok",
        titleKey: "preflight.check.mx_go",
        titleArgs: { reg: mx.registration },
      });
    }
  }

  // 1c. (v8.3) Ubicación del avión: si está en otro aeropuerto que el origen,
  // se ofrece el traslado (si vuela igual, se cobra al cerrar el vuelo).
  const fl = input.fleet;
  if (fl && route && !fl.location.inFlight) {
    const orig = route.originIcao.toUpperCase();
    const loc = fl.location;
    const reg = fl.registration;
    const ferry =
      loc.ferry && Date.parse(loc.ferry.arrivesAt) > Date.now() ? loc.ferry : null;
    const res = loc.reservation;
    // (6.2d) El avión real vuela ahora: no se puede trasladar; se reserva.
    const airborne = !!loc.live?.airborne && !(res && res.landedIcao);
    if (airborne && !res) {
      checks.push({
        id: "ferry",
        status: "warn",
        titleKey: "preflight.check.ferry_airborne",
        titleArgs: { reg, cs: loc.live?.callsign ?? "?" },
        detailKey: "preflight.check.ferry_airborne_detail",
        action: { kind: "reserve", registration: reg },
        actionKey: "preflight.action.reserve",
      });
    } else if (res && res.airborne && !res.landedIcao) {
      checks.push({
        id: "ferry",
        status: "info",
        titleKey: "preflight.check.reserved_airborne",
        titleArgs: { reg },
        detailKey: "preflight.check.reserved_detail",
        detailArgs: { until: fmtDayClock(res.expiresAt) },
        action: { kind: "reserve-cancel", id: res.id },
        actionKey: "preflight.action.reserve_cancel",
      });
    } else if (ferry) {
      const toOrigin = ferry.toIcao === orig;
      checks.push({
        id: "ferry",
        status: toOrigin ? "info" : "warn",
        titleKey: toOrigin
          ? "preflight.check.ferry_enroute"
          : "preflight.check.ferry_enroute_other",
        titleArgs: { reg, to: ferry.toIcao, orig },
        countdownTo: ferry.arrivesAt,
        detailKey: "preflight.check.ferry_enroute_detail",
        detailArgs: { cost: fmtFerryCost(ferry.costTotal) },
        action: { kind: "ferry-cancel", id: ferry.id, toIcao: ferry.toIcao },
        actionKey: "preflight.action.ferry_cancel",
      });
    } else if (loc.icao === orig) {
      checks.push({
        id: "ferry",
        status: "ok",
        titleKey: "preflight.check.ferry_here",
        titleArgs: { reg, icao: orig },
      });
    } else if (loc.icao) {
      const q = fl.origin === orig ? fl.quote : null;
      checks.push({
        id: "ferry",
        status: "warn",
        // (6.2d) Si lo movió un vuelo real, se dice.
        titleKey: loc.real ? "preflight.check.ferry_needed_real" : "preflight.check.ferry_needed",
        titleArgs: { reg, at: loc.icao, orig },
        detailKey: q ? "preflight.check.ferry_needed_detail" : "preflight.check.ferry_no_quote",
        detailArgs: q
          ? { duration: fmtFerryDuration(q.arriveMinutes ?? q.blockMinutes), cost: fmtFerryCost(q.costTotal) }
          : undefined,
        action: q
          ? {
              kind: "ferry",
              registration: reg,
              toIcao: orig,
              model: fl.model,
              cost: fmtFerryCost(q.costTotal),
              duration: fmtFerryDuration(q.arriveMinutes ?? q.blockMinutes),
            }
          : undefined,
        actionKey: q ? "preflight.action.ferry" : undefined,
      });
    }
    // (6.2d) Reserva activa (ya en tierra): el avión no se moverá con vuelos reales.
    if (res && !(res.airborne && !res.landedIcao)) {
      checks.push({
        id: "reservation",
        status: "info",
        titleKey: "preflight.check.reserved",
        titleArgs: { reg },
        detailKey: "preflight.check.reserved_detail",
        detailArgs: { until: fmtDayClock(res.expiresAt) },
        action: { kind: "reserve-cancel", id: res.id },
        actionKey: "preflight.action.reserve_cancel",
      });
    }
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
    // (v6.2.34) Bypass: el usuario marcó "usaré el escenario por
    // defecto" para este ICAO → damos el punto por resuelto.
    if (bypass.has(icao.toUpperCase())) {
      checks.push({
        id: `${leg}_scn`,
        status: "ok",
        titleKey: `preflight.check.${leg}_bypass`,
        titleArgs: { icao },
        bypassIcao: icao.toUpperCase(),
      });
    } else if (!pkg) {
      checks.push({
        id: `${leg}_scn`,
        status: "warn",
        titleKey: `preflight.check.${leg}_default`,
        titleArgs: { icao },
        detailKey: "preflight.check.default_hint",
        action: { kind: "search", icao },
        actionKey: "preflight.action.download",
        bypassIcao: icao.toUpperCase(),
      });
    } else if (pkg.enabled === false) {
      checks.push({
        id: `${leg}_scn`,
        status: "warn",
        titleKey: `preflight.check.${leg}_disabled`,
        titleArgs: { icao },
        action: { kind: "enable", folderName: pkg.folderName },
        actionKey: "preflight.action.enable",
        bypassIcao: icao.toUpperCase(),
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

    // (v6.2.34) GSX: si usas GSX y tienes el escenario instalado pero NO
    // hay perfil GSX para este aeropuerto, ofrecemos buscarlo/descargarlo.
    // Es informativo (no bloquea "listo"), y no aplica si vas por defecto.
    if (
      gsxEnabled &&
      pkg &&
      pkg.enabled !== false &&
      !bypass.has(icao.toUpperCase()) &&
      !gsxIcaos.has(icao.toUpperCase())
    ) {
      checks.push({
        id: `${leg}_gsx`,
        status: "info",
        titleKey: `preflight.check.${leg}_gsx`,
        titleArgs: { icao },
        action: { kind: "gsx", icao },
        actionKey: "preflight.action.gsx",
      });
    }

    // (v6.2.44) GSX: perfil instalado PERO con update disponible en
    // flightsim.to → info con link para re-descargar la versión nueva.
    const gsxUpd = gsxUpdatesMap.get(icao.toUpperCase());
    if (
      gsxEnabled &&
      pkg &&
      pkg.enabled !== false &&
      !bypass.has(icao.toUpperCase()) &&
      gsxIcaos.has(icao.toUpperCase()) &&
      gsxUpd
    ) {
      checks.push({
        id: `${leg}_gsx_upd`,
        status: "info",
        titleKey: `preflight.check.${leg}_gsx_update`,
        titleArgs: { icao, version: gsxUpd.latestVersion ?? "?" },
        action: { kind: "gsx-update", icao, link: gsxUpd.link },
        actionKey: "preflight.action.gsx_update",
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
