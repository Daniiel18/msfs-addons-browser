import { create } from "zustand";
import { api } from "../lib/tauri";
import type { AchievementProgress } from "../lib/types";

/**
 * (v7) Logros. Se calculan en el backend a partir del historial de vuelos
 * (`flight_log`), así que no hay estado persistente aquí: sólo el último
 * resultado + un refresh bajo demanda (al abrir el panel) y tras cada vuelo.
 */
interface AchievementsState {
  list: AchievementProgress[];
  loading: boolean;
  loadedAt: number | null;
  refresh: () => Promise<void>;
  /** Refresca sólo si nunca se cargó o pasó más de `maxAgeMs`. */
  ensureFresh: (maxAgeMs?: number) => void;
}

export const useAchievementsStore = create<AchievementsState>((set, get) => ({
  list: [],
  loading: false,
  loadedAt: null,
  async refresh() {
    if (get().loading) return;
    set({ loading: true });
    try {
      const list = await api.listAchievements();
      set({ list, loadedAt: Date.now() });
    } catch (e) {
      console.warn("listAchievements falló:", e);
    } finally {
      set({ loading: false });
    }
  },
  ensureFresh(maxAgeMs = 60_000) {
    const { loadedAt, refresh } = get();
    if (loadedAt == null || Date.now() - loadedAt > maxAgeMs) void refresh();
  },
}));
