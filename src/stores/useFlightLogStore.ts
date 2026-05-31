import { create } from "zustand";
import type { AirlineTag, FlightLogEntry, FlightStatus } from "../lib/types";
import { api } from "../lib/tauri";

/**
 * Estado del flight log. Las entradas las inserta el watcher de
 * SimConnect (Rust) y la app las pinta en el mapa como líneas
 * sólidas verdes (a diferencia de SimBrief, que usa cyan dashed).
 *
 * El store se suscribe al evento `flightlog://changed` en el
 * bootstrap para refrescar la lista cuando el watcher detecta un
 * despegue/aterrizaje sin necesidad de polling.
 */
interface FlightLogState {
  entries: FlightLogEntry[];
  /** Estado del watcher: ¿MSFS corriendo? ¿qué OFP cuadra como
   *  vuelo activo? El watcher emite eventos al cambiar; lo
   *  guardamos aquí para que la UI lo lea sin polling. */
  status: FlightStatus | null;
  loading: boolean;
  lastError: string | null;

  /** (v3.6.0 Phase H — Epic D) Aerolíneas detectadas en el historial.
   *  Cargadas en el bootstrap + tras cada reload de entries. */
  airlines: AirlineTag[];
  /** (v3.6.0) Tag de aerolínea seleccionada. `null` = sin filtro (todos
   *  los vuelos). Cuando se setea, RoutesMapView atenúa rutas que no
   *  matchean y StatsWidget muestra KPIs específicos. La key es ICAO
   *  cuando está disponible (preferido), sino el `name` para fallback. */
  selectedAirline: { icao: string | null; name: string } | null;
  setSelectedAirline: (
    tag: { icao: string | null; name: string } | null,
  ) => void;

  /** (v3.6.3 fix J2) Progreso del import VAS-ACARS. Global para que
   *  cuando el usuario cierra Settings y la reabre, el botón siga
   *  mostrando la barra de progreso donde iba. `null` = idle. */
  vasImport: {
    running: boolean;
    current: number;
    total: number;
    phase: "started" | "importing" | "done";
  } | null;
  setVasImport: (
    p:
      | {
          running: boolean;
          current: number;
          total: number;
          phase: "started" | "importing" | "done";
        }
      | null,
  ) => void;

  bootstrap: () => Promise<void>;
  reload: () => Promise<void>;
  reloadAirlines: () => Promise<void>;
  remove: (id: number) => Promise<void>;
  /** Helper de testing — inserta un vuelo demo EBBR→LEMD. Disponible
   *  desde la consola para validar la UI sin MSFS corriendo. */
  seedDemoEntry: () => Promise<void>;
}

export const useFlightLogStore = create<FlightLogState>((set, get) => ({
  entries: [],
  status: null,
  loading: false,
  lastError: null,
  airlines: [],
  selectedAirline: null,
  vasImport: null,

  setSelectedAirline(tag) {
    set({ selectedAirline: tag });
  },
  setVasImport(p) {
    set({ vasImport: p });
  },

  async bootstrap() {
    await get().reload();
    await get().reloadAirlines();
    // Cargamos el estado de vuelo actual del watcher.
    api
      .getFlightStatus()
      .then((s) => set({ status: s }))
      .catch((e) => console.warn("flight status fetch failed:", e));
    // Suscripción al evento del watcher — un fire-and-forget
    // listener vive lo que vive la app.
    api
      .onFlightLogChange(() => {
        get()
          .reload()
          .then(() => get().reloadAirlines())
          .catch((e) =>
            console.warn("flightlog reload after change failed:", e),
          );
      })
      .catch((e) => console.warn("flightlog onChange subscribe failed:", e));
    api
      .onFlightStatus((s) => set({ status: s }))
      .catch((e) => console.warn("flight status subscribe failed:", e));
    // (v3.6.3 fix J2) Suscripción al progreso de import VAS-ACARS.
    // El backend emite "vas:import:progress" con { current, total, phase }.
    // Guardamos el state global; el botón en Settings lee de aquí en
    // vez de tener su propio useState — así al cerrar y reabrir el
    // modal, la barra de progreso sigue avanzando.
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<{
        current: number;
        total: number;
        phase: "started" | "importing" | "done";
      }>("vas:import:progress", (event) => {
        const p = event.payload;
        if (p.phase === "done") {
          // Mostrar 100% por 1.5s y luego limpiar.
          set({ vasImport: { running: true, ...p } });
          setTimeout(() => {
            const s = useFlightLogStore.getState();
            if (s.vasImport?.phase === "done") {
              set({ vasImport: null });
            }
          }, 1500);
        } else {
          set({ vasImport: { running: true, ...p } });
        }
      }).catch((e) => console.warn("vas:import:progress subscribe failed:", e));
    });
  },

  async reload() {
    set({ loading: true, lastError: null });
    try {
      const entries = await api.listFlightLog();
      set({ entries, loading: false });
    } catch (e) {
      set({ lastError: String(e), loading: false });
    }
  },

  async reloadAirlines() {
    try {
      const tags = await api.listAirlines();
      set({ airlines: tags });
    } catch (e) {
      console.warn("listAirlines failed:", e);
    }
  },

  async remove(id) {
    try {
      await api.deleteFlightLogEntry(id);
      set((s) => ({ entries: s.entries.filter((e) => e.id !== id) }));
    } catch (e) {
      console.warn("flightlog delete failed:", e);
    }
  },

  async seedDemoEntry() {
    try {
      await api.debugSeedFlightLog();
      await get().reload();
    } catch (e) {
      console.warn("flightlog seed failed:", e);
    }
  },
}));

// Exposición del helper en window para llamarlo desde devtools
// mientras la integración real con SimConnect no está cableada.
if (typeof window !== "undefined") {
  (window as unknown as { __seedFlightLog?: () => Promise<void> }).__seedFlightLog =
    () => useFlightLogStore.getState().seedDemoEntry();
}
