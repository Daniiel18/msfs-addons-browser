import { create } from "zustand";
import { api } from "../lib/tauri";
import type { AircraftLocation, Ferry, FerryQuote } from "../lib/types";

/**
 * (v8.3 · Etapa 6.1) Ubicación de los aviones y traslados para el prevuelo.
 *
 * `load` trae dónde está el avión del OFP y, si está en otro aeropuerto que
 * el origen del plan, cuánto costaría trasladarlo. `epoch` sube con cada
 * traslado pedido/cancelado para que el prevuelo recargue.
 */
export interface FerryInfo {
  location: AircraftLocation;
  /** Traslado desde donde está hasta el origen del plan (si hace falta). */
  quote: FerryQuote | null;
  /** Origen del plan para el que se calculó `quote`. */
  origin: string | null;
}

interface FerryState {
  epoch: number;
  info: Record<string, FerryInfo>;
  load: (registration: string, originIcao: string | null, model: string | null) => Promise<void>;
  request: (registration: string, toIcao: string, model: string | null) => Promise<Ferry>;
  cancel: (id: number) => Promise<void>;
}

/** Igual que `airline_economy::normalize_reg` en Rust. */
export function normalizeFerryReg(reg: string): string {
  return reg.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
}

const inflight = new Set<string>();

export const useFerryStore = create<FerryState>((set, get) => ({
  epoch: 0,
  info: {},
  async load(registration, originIcao, model) {
    const key = normalizeFerryReg(registration);
    const origin = originIcao?.trim().toUpperCase() || null;
    const stamp = `${key}|${origin}|${get().epoch}`;
    if (!key || inflight.has(stamp)) return;
    inflight.add(stamp);
    try {
      const location = await api.aircraftLocation(registration);
      let quote: FerryQuote | null = null;
      if (origin && location.icao && location.icao !== origin && !location.ferry && !location.inFlight) {
        quote = await api
          .ferryQuote(location.icao, origin, model ?? location.model)
          .catch(() => null);
      }
      set((s) => ({ info: { ...s.info, [key]: { location, quote, origin } } }));
    } catch {
      /* sin ubicación: el prevuelo lo omite */
    } finally {
      inflight.delete(stamp);
    }
  },
  async request(registration, toIcao, model) {
    const ferry = await api.ferryRequest(registration, toIcao, model);
    set((s) => ({ epoch: s.epoch + 1 }));
    return ferry;
  },
  async cancel(id) {
    await api.ferryCancel(id);
    set((s) => ({ epoch: s.epoch + 1 }));
  },
}));
