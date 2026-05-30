import { create } from "zustand";
import type { FlightLogEntry, FlightStatus } from "../lib/types";
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

  bootstrap: () => Promise<void>;
  reload: () => Promise<void>;
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

  async bootstrap() {
    await get().reload();
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
          .catch((e) =>
            console.warn("flightlog reload after change failed:", e),
          );
      })
      .catch((e) => console.warn("flightlog onChange subscribe failed:", e));
    api
      .onFlightStatus((s) => set({ status: s }))
      .catch((e) => console.warn("flight status subscribe failed:", e));
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
