import { create } from "zustand";
import { api } from "../lib/tauri";
import type { AiracUpdateInfo } from "../lib/types";

/**
 * (v6.2.8) Estado de actualización del AIRAC (datos de navegación). El backend
 * `airac_check_update` calcula el ciclo vigente del calendario fijo de 28 días y
 * lo compara con el instalado. La app consulta al arrancar y cada hora; se
 * comparte aquí para Notificaciones + Dashboard.
 *
 * Al actualizar (clic) abrimos Navigraph Hub (ruta resuelta por usuario en el
 * backend; si no se halla, abrimos la web) y marcamos el ciclo como instalado
 * para no re-avisar. La notificación PERSISTE hasta que se actualiza (no es
 * descartable), igual que la de GSX.
 */

const NAVIGRAPH_URL = "https://navigraph.com/";

interface AiracUpdateState {
  info: AiracUpdateInfo | null;
  check: () => Promise<void>;
  /** Abre Navigraph Hub (o la web) y marca el ciclo vigente como instalado. */
  openUpdater: () => Promise<void>;
}

export const useAiracUpdateStore = create<AiracUpdateState>((set, get) => ({
  info: null,

  check: async () => {
    try {
      const info = await api.airacCheckUpdate();
      set({ info });
    } catch (e) {
      console.warn("airacCheckUpdate falló:", e);
    }
  },

  openUpdater: async () => {
    const info = get().info;
    try {
      const path = await api.airacUpdaterPath();
      if (path) {
        await api.openLocalPath(path);
      } else {
        await api.openExternal(NAVIGRAPH_URL);
      }
    } catch (e) {
      console.warn("no pude abrir Navigraph Hub:", e);
      try {
        await api.openExternal(NAVIGRAPH_URL);
      } catch {
        /* ignore */
      }
    }
    // Marca el ciclo vigente como instalado → borra el aviso (hasUpdate=false).
    if (info?.latestCycle) {
      try {
        await api.airacSetInstalledCycle(info.latestCycle);
      } catch {
        /* ignore */
      }
      set({
        info: {
          ...info,
          installedCycle: info.latestCycle,
          hasUpdate: false,
        },
      });
    }
  },
}));

/** ¿Hay update de AIRAC que mostrar? Persiste hasta actualizar (no descartable). */
export function airacUpdateVisible(info: AiracUpdateInfo | null): boolean {
  return !!info?.hasUpdate;
}
