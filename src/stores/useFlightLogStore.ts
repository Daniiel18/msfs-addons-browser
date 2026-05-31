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
    // (v3.6.3 fix J2 → v3.6.4 fix K4) Suscripción al progreso de import.
    // Backend emite "vas:import:progress" con { current, total, phase,
    // imported?, updated?, skipped? }. Al "done" disparamos:
    //   · Notificación nativa Windows (Web Notification API — no
    //     requiere plugin, anda en el WebView). Útil cuando el
    //     usuario está focus en MSFS u otra app.
    //   · Toast in-app via useToastStore.
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<{
        current: number;
        total: number;
        phase: "started" | "importing" | "done";
        imported?: number;
        updated?: number;
        skipped?: number;
      }>("vas:import:progress", (event) => {
        const p = event.payload;
        if (p.phase === "done") {
          set({ vasImport: { running: true, ...p } });
          // Pedir permiso lazy (la primera vez) y disparar la nativa.
          void notifyImportDone(p.imported ?? 0, p.updated ?? 0);
          // Toast in-app — usar dynamic import para evitar circular
          // dependency con useToastStore.
          import("./useToastStore").then(({ useToastStore }) => {
            useToastStore.getState().push({
              kind: "success",
              title: "VAS-ACARS",
              message: `${p.imported ?? 0} nuevos · ${p.updated ?? 0} actualizados`,
              ttlMs: 5000,
            });
          });
          // Auto-limpiar la barra de progreso tras 1.5s.
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
      // (v3.6.4 fix K3) Refresca airlines — si era el último vuelo de
      // esa aerolínea, el chip debe desaparecer. Antes los tags se
      // quedaban persistentes tras borrar todos los vuelos.
      await get().reloadAirlines();
      // Si la aerolínea seleccionada ya no existe en el listado,
      // desactiva el filtro automáticamente.
      const sel = get().selectedAirline;
      if (sel) {
        const stillExists = get().airlines.some((a) =>
          sel.icao ? a.icao === sel.icao : a.name === sel.name,
        );
        if (!stillExists) set({ selectedAirline: null });
      }
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

/**
 * (v3.6.4 fix K4) Notificación nativa de Windows cuando termina el
 * import de VAS-ACARS. Usa la Web Notification API que el WebView
 * de Tauri expone gratis — no requiere instalar el plugin
 * `tauri-plugin-notification`.
 *
 * El permiso se pide la primera vez que se intenta notificar; una
 * vez aceptado, Windows lo recuerda y las notificaciones aparecen en
 * el centro de notificaciones aunque la app esté minimizada.
 */
async function notifyImportDone(imported: number, updated: number) {
  if (typeof window === "undefined" || !("Notification" in window)) {
    return;
  }
  try {
    if (Notification.permission === "default") {
      // Pedir permiso silenciosamente — si el usuario rechaza, no
      // volvemos a molestar.
      await Notification.requestPermission().catch(() => {});
    }
    if (Notification.permission !== "granted") {
      return;
    }
    const title = "SimFleet — Importación completa";
    const body =
      imported === 0 && updated === 0
        ? "No había vuelos nuevos para importar."
        : `${imported} vuelos importados · ${updated} actualizados`;
    new Notification(title, {
      body,
      tag: "simfleet-vas-import-done", // reemplaza notificación previa si existe
    });
  } catch (e) {
    console.warn("notifyImportDone failed:", e);
  }
}
