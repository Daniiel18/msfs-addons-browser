import { create } from "zustand";
import type { SimBriefFlight } from "../lib/types";
import { api } from "../lib/tauri";

/**
 * Estado SimBrief: pilotId configurado + historial de vuelos
 * persistido en SQLite + flags de carga.
 *
 * El bootstrap se llama una vez al montar `App`. Si hay pilotId
 * configurado, también dispara un refresh en background para que
 * el último OFP esté disponible al instante.
 */
interface SimBriefState {
  pilotId: string | null;
  flights: SimBriefFlight[];
  refreshing: boolean;
  lastError: string | null;

  bootstrap: () => Promise<void>;
  setPilotId: (id: string) => Promise<void>;
  refresh: () => Promise<void>;
  reload: () => Promise<void>;
  remove: (ofpId: string) => Promise<void>;
}

export const useSimBriefStore = create<SimBriefState>((set, get) => ({
  pilotId: null,
  flights: [],
  refreshing: false,
  lastError: null,

  async bootstrap() {
    try {
      const [pilotId, flights] = await Promise.all([
        api.getSimbriefPilotId(),
        api.listSimbriefFlights(),
      ]);
      set({ pilotId, flights });
      if (pilotId) {
        // Background — no bloquea bootstrap si la API está caída.
        get().refresh().catch((e) => console.warn("simbrief refresh bg:", e));
      }
    } catch (e) {
      console.warn("simbriefStore.bootstrap failed:", e);
    }
  },

  async setPilotId(id) {
    set({ lastError: null });
    try {
      await api.setSimbriefPilotId(id);
      set({ pilotId: id });
      await get().refresh();
    } catch (e) {
      set({ lastError: String(e) });
    }
  },

  async refresh() {
    if (get().refreshing) return;
    if (!get().pilotId) {
      set({ lastError: "Configura tu SimBrief Pilot ID primero." });
      return;
    }
    set({ refreshing: true, lastError: null });
    try {
      await api.refreshSimbrief();
      const flights = await api.listSimbriefFlights();
      set({ flights });
    } catch (e) {
      set({ lastError: String(e) });
    } finally {
      set({ refreshing: false });
    }
  },

  async reload() {
    try {
      const flights = await api.listSimbriefFlights();
      set({ flights });
    } catch (e) {
      console.warn("simbrief reload failed:", e);
    }
  },

  async remove(ofpId) {
    try {
      await api.deleteSimbriefFlight(ofpId);
      set((s) => ({ flights: s.flights.filter((f) => f.ofpId !== ofpId) }));
    } catch (e) {
      console.warn("simbrief delete failed:", e);
    }
  },
}));
