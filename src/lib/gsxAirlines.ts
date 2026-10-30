import { canonicalAirlineKey, iataToIcao, icaoToIata, icaoToName } from "./airlineCodes";
import type { GsxParking } from "./types";

/**
 * (26/10) Aerolíneas de los gates de un perfil de GSX (`airlinecodes`).
 *
 * Los perfiles mezclan códigos ICAO ("DAL"), IATA ("WS") y agentes de
 * handling ("SWISSPORT", "MENZ"). Cuentan como aerolínea los de 3 letras y los
 * IATA de 2 caracteres que conocemos (con nombre o logo); el resto se ignora.
 * La clave es la misma que usa el filtro del FlightBook (`canonicalAirlineKey`).
 */
export interface GsxAirline {
  key: string;
  icao: string;
  name: string;
}

export function gateAirlines(p: GsxParking): GsxAirline[] {
  const out = new Map<string, GsxAirline>();
  for (const raw of p.airlines ?? []) {
    const code = raw.toUpperCase().trim();
    const icao = /^[A-Z]{3}$/.test(code) ? code : /^[A-Z0-9]{2}$/.test(code) ? iataToIcao(code) : null;
    if (!icao) continue;
    const name = icaoToName(icao);
    if (!name && !icaoToIata(icao)) continue;
    const key = canonicalAirlineKey(icao, null);
    if (!out.has(key)) out.set(key, { key, icao, name: name ?? icao });
  }
  return [...out.values()];
}

/** Chips del perfil: cada aerolínea con cuántos gates tiene, de más a menos. */
export function airlineChips(parkings: GsxParking[]): (GsxAirline & { count: number })[] {
  const byKey = new Map<string, GsxAirline & { count: number }>();
  for (const p of parkings) {
    for (const a of gateAirlines(p)) {
      const prev = byKey.get(a.key);
      if (prev) prev.count++;
      else byKey.set(a.key, { ...a, count: 1 });
    }
  }
  return [...byKey.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

/** Clave para cruzar el gate de un vuelo con el del perfil ("Gate 40" ≈ "gate40"). */
export const gateKey = (s: string) => s.toLowerCase().replace(/\s+/g, "");
