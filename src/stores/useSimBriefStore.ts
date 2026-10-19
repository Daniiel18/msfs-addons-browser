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
  /** (19/10) Todos los OFP de la cuenta compartida — SOLO para el plan del
   *  rival en Crew VS. El resto de la app usa `flights` (sólo los míos). */
  allFlights: SimBriefFlight[];
  refreshing: boolean;
  lastError: string | null;

  bootstrap: () => Promise<void>;
  setPilotId: (id: string) => Promise<void>;
  refresh: () => Promise<void>;
  reload: () => Promise<void>;
  remove: (ofpId: string) => Promise<void>;
  /** (19/10) "No es mío": lo excluye para siempre y recarga la lista. */
  excludeOfp: (ofpId: string) => Promise<void>;
}

export const useSimBriefStore = create<SimBriefState>((set, get) => ({
  pilotId: null,
  flights: [],
  allFlights: [],
  refreshing: false,
  lastError: null,

  async bootstrap() {
    try {
      const [pilotId, flights, allFlights] = await Promise.all([
        api.getSimbriefPilotId(),
        api.listSimbriefFlights(),
        api.listSimbriefFlightsAll().catch(() => [] as SimBriefFlight[]),
      ]);
      set({ pilotId, flights, allFlights });
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
      await get().reload();
    } catch (e) {
      set({ lastError: String(e) });
    } finally {
      set({ refreshing: false });
    }
  },

  async reload() {
    try {
      const [flights, allFlights] = await Promise.all([
        api.listSimbriefFlights(),
        api.listSimbriefFlightsAll().catch(() => [] as SimBriefFlight[]),
      ]);
      set({ flights, allFlights });
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

  async excludeOfp(ofpId) {
    try {
      await api.simbriefExcludeOfp(ofpId);
      set((s) => ({ flights: s.flights.filter((f) => f.ofpId !== ofpId) }));
      await get().reload();
    } catch (e) {
      console.warn("simbrief exclude failed:", e);
    }
  },
}));
