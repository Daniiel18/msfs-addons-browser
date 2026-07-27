import { create } from "zustand";
import { api } from "../lib/tauri";
import type { FlightsimUpdate } from "../lib/types";

/**
 * (v6.2.60) Updates de addons descargados de flightsim.to por el navegador
 * embebido. Forward-only y FIABLE: al instalar guardamos el `file_id` +
 * `updatedAt` (baseline) en el backend; aquí sólo pedimos el resultado del
 * chequeo (compara el updatedAt actual contra el baseline). Indexado por
 * `folderName` (= carpeta de Community) para casar el badge en AddonsView.
 *
 * Se refresca en el bootstrap de la app y tras instalar algo del embebido.
 */
interface FlightsimUpdateState {
  /** folderName → info de update, SÓLO los que tienen `hasUpdate`. */
  updates: Map<string, FlightsimUpdate>;
  refreshUpdates: () => Promise<void>;
  /** Marca un addon como manejado: quita el badge al instante y avanza el
   *  baseline en el backend (se llama al pulsar el badge). */
  ackUpdate: (folderName: string) => void;
}

export const useFlightsimUpdateStore = create<FlightsimUpdateState>((set, get) => ({
  updates: new Map<string, FlightsimUpdate>(),
  async refreshUpdates() {
    try {
      const list = await api.flightsimCheckUpdates();
      const map = new Map<string, FlightsimUpdate>();
      for (const u of list) {
        if (u.hasUpdate) map.set(u.folderName, u);
      }
      set({ updates: map });
    } catch (e) {
      console.warn("flightsimCheckUpdates falló:", e);
    }
  },
  ackUpdate(folderName) {
    const next = new Map(get().updates);
    if (next.delete(folderName)) set({ updates: next });
    void api.flightsimAckUpdate(folderName);
  },
}));
