import { create } from "zustand";
import { api } from "../lib/tauri";

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
 */
interface GsxLocalState {
  /** Set de ICAOs con al menos un perfil .ini en el folder. */
  installedIcaos: Set<string>;
  refresh: () => Promise<void>;
}

export const useGsxLocalStore = create<GsxLocalState>((set) => ({
  installedIcaos: new Set<string>(),
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
}));
