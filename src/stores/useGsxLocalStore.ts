import { create } from "zustand";
import { api } from "../lib/tauri";
import type { GsxProfileUpdate } from "../lib/types";

/**
 * (v1.1.4) Store mínimo para la lista de ICAOs con perfil GSX
 * instalado localmente en `%APPDATA%\Virtuali\GSX\MSFS`. La consume
 * `ResultCard` y `PackageDetailModal` para mostrar un check al lado
 * del ICAO del escenario cuando el usuario ya tiene su perfil GSX
 * matching.
 *
 * Se hidrata al arrancar la app (`refresh()` en el bootstrap) y se
 * refresca tras un install (`gsxInstallProfile` actualiza la lista).
 * No hace polling — los perfiles no aparecen solos, los pone el
 * usuario o esta misma app.
 *
 * (v6.2.44) Además guarda `updates`: por cada ICAO instalado, si
 * flightsim.to publicó una versión más nueva que el .ini local. El
 * badge del mapa pasa de verde a ámbar (clickeable → link del perfil)
 * y el pre-vuelo lo indica con el link de descarga.
 */
interface GsxLocalState {
  /** Set de ICAOs con al menos un perfil .ini en el folder. */
  installedIcaos: Set<string>;
  /** ICAO(upper) → info de update, SÓLO los que tienen `hasUpdate`. */
  updates: Map<string, GsxProfileUpdate>;
  refresh: () => Promise<void>;
  /** Chequea updates de los perfiles instalados (lento: pega a la red
   *  por cada ICAO). Se llama en background tras el `refresh()`. */
  refreshUpdates: () => Promise<void>;
}

export const useGsxLocalStore = create<GsxLocalState>((set) => ({
  installedIcaos: new Set<string>(),
  updates: new Map<string, GsxProfileUpdate>(),
  async refresh() {
    try {
      const list = await api.gsxListInstalledIcaos();
      set({
        installedIcaos: new Set(list.map((s) => s.toUpperCase())),
      });
    } catch (e) {
      console.warn("gsxListInstalledIcaos falló:", e);
    }
  },
  async refreshUpdates() {
    try {
      const list = await api.gsxCheckProfileUpdates();
      const map = new Map<string, GsxProfileUpdate>();
      for (const u of list) {
        if (u.hasUpdate) map.set(u.icao.toUpperCase(), u);
      }
      set({ updates: map });
    } catch (e) {
      console.warn("gsxCheckProfileUpdates falló:", e);
    }
  },
}));
