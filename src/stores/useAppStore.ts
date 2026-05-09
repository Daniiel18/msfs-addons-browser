import { create } from "zustand";
import type { Addon, SourceDescriptor } from "../lib/types";
import { api } from "../lib/tauri";

type Status = "idle" | "loading" | "success" | "error";

/** Vista activa en `App`. Dashboard (default home, totales) /
 *  Buscar / Mapa (sólo escenarios) / Addons (resto: aircraft,
 *  livery, sound, etc). */
export type View = "dashboard" | "search" | "map" | "addons";

interface AppState {
  sources: SourceDescriptor[];
  activeSourceId: string;
  query: string;
  results: Addon[];
  status: Status;
  error: string | null;
  view: View;

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
}

export const useAppStore = create<AppState>((set, get) => ({
  sources: [],
  activeSourceId: "sceneryaddons",
  query: "",
  results: [],
  status: "idle",
  error: null,
  view: "dashboard",
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
      set((s) => ({
        catalogCache: {
          ...s.catalogCache,
          [sourceId]: { addons: res.addons, hasMore: res.hasMore },
        },
      }));
    } catch (e) {
      console.warn("preloadCatalog failed:", sourceId, e);
    }
  },
  setQuery: (query) => set({ query }),
  setResults: (results) => set({ results, browseMode: "search" }),
  setStatus: (status) => set({ status }),
  setError: (error) => set({ error }),
  setView: (view) => set({ view }),

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
