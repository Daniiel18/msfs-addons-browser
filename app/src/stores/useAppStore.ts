import { create } from "zustand";
import type { Addon, SourceDescriptor } from "../lib/types";
import { api } from "../lib/tauri";

type Status = "idle" | "loading" | "success" | "error";

/** Vista activa en `App`. Buscar / Mapa (sólo escenarios) /
 *  Addons (resto: aircraft, livery, sound, etc). */
export type View = "search" | "map" | "addons";

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

  setSources: (s: SourceDescriptor[]) => void;
  setActiveSource: (id: string) => void;
  setQuery: (q: string) => void;
  setResults: (r: Addon[]) => void;
  setStatus: (s: Status) => void;
  setError: (e: string | null) => void;
  setView: (v: View) => void;

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
  view: "search",
  browsePage: 1,
  browseHasMore: false,
  browseMode: "browse",

  setSources: (sources) => set({ sources }),
  setActiveSource: (activeSourceId) => {
    set({
      activeSourceId,
      query: "",
      results: [],
      status: "idle",
      error: null,
      browsePage: 1,
      browseHasMore: false,
      browseMode: "browse",
    });
    // Cargar el catálogo de la nueva fuente automáticamente.
    void get().loadBrowsePage(1);
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
    } catch (e) {
      set({ error: String(e), status: "error" });
    }
  },
}));
