import type { FlightLogEntry } from "./types";

/**
 * (v6.2.30 / R7) Estadísticas anuales estilo "Wrapped" derivadas del
 * FlightBook. Puro y testeable — sólo agrega sobre los vuelos
 * completados del año pedido.
 */

export interface WrappedTop {
  key: string;
  label: string;
  count: number;
}

export interface WrappedStats {
  year: number;
  flights: number;
  hours: number;
  distanceNm: number;
  airportsVisited: number;
  longestNm: number;
  bestLandingFpm: number | null;
  topDestinations: WrappedTop[];
  topAircraft: WrappedTop[];
  busiestMonth: { month: number; count: number } | null;
}

function topN(counts: Map<string, { label: string; n: number }>, n: number): WrappedTop[] {
  return [...counts.entries()]
    .map(([key, v]) => ({ key, label: v.label, count: v.n }))
    .sort((a, b) => b.count - a.count)
    .slice(0, n);
}

export function computeWrapped(
  entries: FlightLogEntry[],
  year: number,
): WrappedStats {
  const flights = entries.filter((e) => {
    if (e.status && e.status !== "completed") return false;
    if (!e.startedAt) return false;
    const d = new Date(e.startedAt);
    return !isNaN(d.getTime()) && d.getFullYear() === year;
  });

  let hoursS = 0;
  let distanceNm = 0;
  let longestNm = 0;
  let bestLandingFpm: number | null = null;
  const airports = new Set<string>();
  const dests = new Map<string, { label: string; n: number }>();
  const aircraft = new Map<string, { label: string; n: number }>();
  const months = new Array<number>(12).fill(0);

  for (const e of flights) {
    if (e.flightTimeS) hoursS += e.flightTimeS;
    if (e.distanceNm) {
      distanceNm += e.distanceNm;
      longestNm = Math.max(longestNm, e.distanceNm);
    }
    if (e.originIcao) airports.add(e.originIcao.toUpperCase());
    if (e.destinationIcao) {
      const k = e.destinationIcao.toUpperCase();
      airports.add(k);
      const prev = dests.get(k);
      dests.set(k, {
        label: e.destinationName || e.destinationIcao,
        n: (prev?.n ?? 0) + 1,
      });
    }
    const acKey = (e.aircraftModel || e.aircraftAtcType || e.aircraftTitle || "")
      .trim();
    if (acKey) {
      const prev = aircraft.get(acKey.toUpperCase());
      aircraft.set(acKey.toUpperCase(), {
        label: e.aircraftTitle || e.aircraftModel || acKey,
        n: (prev?.n ?? 0) + 1,
      });
    }
    // Mejor aterrizaje: descenso más suave (|fpm| menor) pero real
    // (negativo; ignoramos rebotes/positivos y ceros dudosos).
    if (e.landingFpm != null && e.landingFpm < 0) {
      if (bestLandingFpm == null || Math.abs(e.landingFpm) < Math.abs(bestLandingFpm)) {
        bestLandingFpm = e.landingFpm;
      }
    }
    const md = new Date(e.startedAt);
    if (!isNaN(md.getTime())) months[md.getMonth()] += 1;
  }

  let busiestMonth: { month: number; count: number } | null = null;
  months.forEach((c, m) => {
    if (c > 0 && (!busiestMonth || c > busiestMonth.count)) {
      busiestMonth = { month: m, count: c };
    }
  });

  return {
    year,
    flights: flights.length,
    hours: hoursS / 3600,
    distanceNm,
    airportsVisited: airports.size,
    longestNm,
    bestLandingFpm,
    topDestinations: topN(dests, 5),
    topAircraft: topN(aircraft, 3),
    busiestMonth,
  };
}
