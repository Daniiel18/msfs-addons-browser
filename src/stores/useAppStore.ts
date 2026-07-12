import { create } from "zustand";
import type { Addon, SourceDescriptor } from "../lib/types";
import { api } from "../lib/tauri";

type Status = "idle" | "loading" | "success" | "error";

/** Vista activa en `App`. Dashboard (default home, totales) /
 *  Buscar / Mapa (sólo escenarios) / Addons (resto: aircraft,
 *  livery, sound, etc) / FlightBook (historial de vuelos reales
 *  con métricas de SimConnect). */
export type View =
  | "dashboard"
  | "search"
  | "map"
  | "addons"
  | "flightbook"
  | "hangar"
  | "economy";

interface AppState {
  sources: SourceDescriptor[];
  activeSourceId: string;
  query: string;
  results: Addon[];
  status: Status;
  error: string | null;
  view: View;

  /** (v6.1) Aerolínea enfocada en la pantalla de Finanzas. `null` = mostrar
   *  todas. La fija `openEconomy` cuando se clica una aerolínea en el Hangar. */
  economyFocus: string | null;
  /** (v6.1) Matrícula para abrir DIRECTO el mantenimiento (clic en el Hangar).
   *  La consume EconomyView una vez y se limpia. */
  economyMaintReg: string | null;

  /** Página del browse-mode (cuando el query está vacío). */
  browsePage: number;
  /** ¿Hay más páginas? Lo devuelve la última respuesta de browse. */
  browseHasMore: boolean;
  /** "browse" cuando los `results` son del catálogo paginado;
   *  "search" cuando vienen de un query del usuario. La UI usa esto
   *  para decidir si pintar paginador o mensaje "Sin resultados". */
  browseMode: "search" | "browse";

  /** Cache de la página 1 del catálogo de cada fuente — la
   *  precargamos en el splash para que cambiar de
   *  SceneryAddons↔Simplaza sea instantáneo y no dispare otra
   *  request HTTP. Clave = sourceId. */
  catalogCache: Record<string, { addons: Addon[]; hasMore: boolean }>;

  setSources: (s: SourceDescriptor[]) => void;
  setActiveSource: (id: string) => void;
  setQuery: (q: string) => void;
  setResults: (r: Addon[]) => void;
  setStatus: (s: Status) => void;
  setError: (e: string | null) => void;
  setView: (v: View) => void;
  /** (v6.1) Abre la pantalla de Finanzas. Con `airlineKey` la enfoca en esa
   *  aerolínea (ICAO o clave); sin argumento muestra todas. Con `maintReg`
   *  abre directo el mantenimiento de esa matrícula. */
  openEconomy: (airlineKey?: string | null, maintReg?: string | null) => void;
  /** (v6.1) Limpia la matrícula de mantenimiento tras consumirla. */
  clearEconomyMaintReg: () => void;
  /** Pre-carga la página 1 del catálogo de una fuente. Idempotente:
   *  si ya está cacheado, no toca la red. */
  preloadCatalog: (sourceId: string) => Promise<void>;

  /** Cambia a la vista de búsqueda, fija el query, y dispara la
   *  búsqueda. Usada desde el panel de notificaciones cuando el
   *  usuario clica una update — queremos llevarlo al resultado
   *  con sus métodos de descarga ya cargados. */
  triggerSearch: (query: string, sourceId?: string) => Promise<void>;
  /** Carga la página `n` (1-based) del catálogo de la fuente activa. */
  loadBrowsePage: (page: number) => Promise<void>;

  /** (v6.2.27 / R4) Semilla de filtro para la pestaña Addons — la fija
   *  la paleta Ctrl+K al saltar a un paquete instalado. AddonsView la
   *  consume una vez (setFilter) y la limpia. `null` = sin semilla. */
  addonsSeedFilter: string | null;
  /** Salta a la pestaña Addons con un filtro pre-cargado. */
  jumpToAddon: (filter: string) => void;
  /** Limpia la semilla tras consumirla. */
  clearAddonsSeed: () => void;

  /** (v6.2.35) Petición de abrir el Crew VS de un vuelo concreto — la fija
   *  la campanita al clicar el aviso "Crew VS listo". FlightBookView la
   *  consume: selecciona el vuelo que casa origen/destino/fecha y abre el
   *  modal de combate. `null` = sin petición. */
  crewVsRequest: { origin: string; dest: string; at: number } | null;
  /** Salta al FlightBook y pide abrir el Crew VS del vuelo que casa. */
  requestCrewVs: (origin: string, dest: string, at: number) => void;
  /** Limpia la petición tras consumirla. */
  clearCrewVsRequest: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  sources: [],
  activeSourceId: "sceneryaddons",
  query: "",
  results: [],
  status: "idle",
  error: null,
  view: "dashboard",
  economyFocus: null,
  economyMaintReg: null,
  browsePage: 1,
  browseHasMore: false,
  browseMode: "browse",
  catalogCache: {},

  setSources: (sources) => set({ sources }),
  setActiveSource: (activeSourceId) => {
    const cached = get().catalogCache[activeSourceId];
    set({
      activeSourceId,
      query: "",
      // Si tenemos pre-cargado el catálogo de esta fuente, lo
      // mostramos al instante sin HTTP. Si no, queda vacío y
      // disparamos browse abajo.
      results: cached ? cached.addons : [],
      status: cached ? "success" : "idle",
      error: null,
      browsePage: 1,
      browseHasMore: cached ? cached.hasMore : false,
      browseMode: "browse",
    });
    // Sólo si no había cache disparamos browse — evita el doble
    // request al alternar entre fuentes.
    if (!cached) {
      void get().loadBrowsePage(1);
    }
  },
  async preloadCatalog(sourceId) {
    if (get().catalogCache[sourceId]) return;
    try {
      const res = await api.browseSource(sourceId, 1);
      set((s) => {
        const isActiveAndIdle =
          s.activeSourceId === sourceId &&
          s.status === "idle" &&
          s.query === "" &&
          s.results.length === 0;
        return {
          catalogCache: {
            ...s.catalogCache,
            [sourceId]: { addons: res.addons, hasMore: res.hasMore },
          },
          // Bug fix v0.1.9: el bootstrap llama `preloadCatalog` por
          // cada fuente, pero el `setActiveSource` no se dispara si
          // el `activeSourceId` ya coincide con el default
          // ("sceneryaddons"). Resultado: `status` queda en "idle"
          // y la pestaña "Buscar" muestra el spinner "Cargando
          // catálogo…" eternamente hasta que el usuario clica otra
          // pestaña/source. Ahora si esta source es la activa y
          // todavía estamos idle, seedeamos los resultados visibles.
          ...(isActiveAndIdle
            ? {
                results: res.addons,
                status: "success" as const,
                browseHasMore: res.hasMore,
                browseMode: "browse" as const,
              }
            : {}),
        };
      });
    } catch (e) {
      console.warn("preloadCatalog failed:", sourceId, e);
    }
  },
  setQuery: (query) => set({ query }),
  setResults: (results) => set({ results, browseMode: "search" }),
  setStatus: (status) => set({ status }),
  setError: (error) => set({ error }),
  setView: (view) => set({ view }),
  openEconomy: (airlineKey = null, maintReg = null) =>
    set({ view: "economy", economyFocus: airlineKey, economyMaintReg: maintReg }),
  clearEconomyMaintReg: () => set({ economyMaintReg: null }),

  addonsSeedFilter: null,
  jumpToAddon: (filter) => set({ view: "addons", addonsSeedFilter: filter }),
  clearAddonsSeed: () => set({ addonsSeedFilter: null }),

  crewVsRequest: null,
  requestCrewVs: (origin, dest, at) =>
    set({ view: "flightbook", crewVsRequest: { origin, dest, at } }),
  clearCrewVsRequest: () => set({ crewVsRequest: null }),

  async triggerSearch(query, sourceId) {
    const targetSource = sourceId ?? get().activeSourceId;
    set({
      view: "search",
      activeSourceId: targetSource,
      query,
      status: "loading",
      error: null,
      results: [],
      browseMode: "search",
    });
    try {
      const r = await api.search(query, targetSource);
      set({ results: r, status: "success" });
    } catch (e) {
      set({ error: String(e), status: "error" });
    }
  },

  async loadBrowsePage(page) {
    const targetSource = get().activeSourceId;
    // Si pedimos la página 1 y la tenemos pre-cargada, sirvela del
    // cache — no hagamos HTTP. Esto evita el "se quedó cargando"
    // al cambiar de SA→Simplaza→SA.
    if (page === 1) {
      const cached = get().catalogCache[targetSource];
      if (cached) {
        set({
          results: cached.addons,
          status: "success",
          browseHasMore: cached.hasMore,
          browseMode: "browse",
          browsePage: 1,
          error: null,
        });
        return;
      }
    }
    set({
      status: "loading",
      error: null,
      browseMode: "browse",
      browsePage: page,
    });
    try {
      const res = await api.browseSource(targetSource, page);
      set({
        results: res.addons,
        status: "success",
        browseHasMore: res.hasMore,
        browseMode: "browse",
      });
      // Actualizamos cache en página 1 — los resultados pueden haber
      // cambiado entre el preload y ahora.
      if (page === 1) {
        set((s) => ({
          catalogCache: {
            ...s.catalogCache,
            [targetSource]: { addons: res.addons, hasMore: res.hasMore },
          },
        }));
      }
    } catch (e) {
      set({ error: String(e), status: "error" });
    }
  },
}));
