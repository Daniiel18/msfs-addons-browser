import { create } from "zustand";
import { api } from "../lib/tauri";
import type { SimAirport } from "../lib/types";

/** (05/11) Aeropuertos que trae el propio simulador. Que uno no esté aquí no
 *  prueba que no exista: es lo que se ve en sus carpetas. */
interface SimContentState {
  airports: SimAirport[];
  icaos: Set<string>;
  load: () => Promise<void>;
}

export const useSimContentStore = create<SimContentState>((set) => ({
  airports: [],
  icaos: new Set<string>(),
  async load() {
    try {
      const airports = await api.simAirports();
      set({ airports, icaos: new Set(airports.map((a) => a.icao)) });
    } catch (e) {
      console.warn("simAirports falló:", e);
    }
  },
}));
