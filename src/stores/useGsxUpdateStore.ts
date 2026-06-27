import { create } from "zustand";
import { api } from "../lib/tauri";
import type { GsxUpdateInfo } from "../lib/types";
import { useFlightLogStore } from "./useFlightLogStore";

/**
 * (v6.2.4) Estado de actualización de GSX (FSDreamTeam couatl). El backend
 * `gsx_check_update` parsea las notas de FSDT y compara con la versión
 * instalada. La app consulta cada hora; el resultado se comparte aquí para
 * pintarlo en Notificaciones y en el Dashboard.
 *
 * Al actualizar (clic en la notificación) abrimos el "FSDT Installer" y marcamos
 * la última versión como instalada, para no volver a avisar por la misma.
 */

// Acceso directo del menú inicio al instalador de FSDT (Windows).
const FSDT_INSTALLER_LNK =
  "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\FSDreamTeam\\FSDT Installer.lnk";

const DISMISSED_KEY = "simfleet:gsx-update-dismissed-version";

function loadDismissed(): string | null {
  try {
    return typeof localStorage !== "undefined"
      ? localStorage.getItem(DISMISSED_KEY)
      : null;
  } catch {
    return null;
  }
}

interface GsxUpdateState {
  info: GsxUpdateInfo | null;
  dismissedVersion: string | null;
  /** (v6.2.6) `true` mientras esperamos a que el usuario CIERRE el sim para
   *  poder actualizar GSX. Mientras tanto el gate (warning) está visible y NO
   *  se abre el instalador ni se borra la notificación. */
  pendingInstall: boolean;
  /** Consulta el backend (llamado al arranque y cada hora). */
  check: () => Promise<void>;
  /** (v6.2.6) Intenta actualizar. Si el sim está ABIERTO → activa el gate
   *  (warning) y espera; si está cerrado → abre el instalador directamente. */
  requestInstall: () => void;
  /** Abre el FSDT Installer, marca la versión como instalada y borra el aviso.
   *  Lo llama el gate al detectar el sim cerrado (o `requestInstall` si no había
   *  sim abierto). */
  performInstall: () => Promise<void>;
  /** Cancela el gate (mantiene la notificación para reintentar). */
  cancelInstall: () => void;
  /** Oculta el aviso para la versión actual (no vuelve hasta una más nueva). */
  dismiss: () => void;
}

export const useGsxUpdateStore = create<GsxUpdateState>((set, get) => ({
  info: null,
  dismissedVersion: loadDismissed(),
  pendingInstall: false,

  check: async () => {
    try {
      const info = await api.gsxCheckUpdate();
      set({ info });
    } catch (e) {
      console.warn("gsxCheckUpdate falló:", e);
    }
  },

  requestInstall: () => {
    // (v6.2.6) GSX NO se puede actualizar con el sim abierto (couatl está en
    // uso). Si lo detectamos corriendo, abrimos el gate (warning) y esperamos a
    // que el usuario lo cierre; el GsxUpdateGate dispara performInstall cuando
    // `simRunning` pase a false.
    const simRunning =
      useFlightLogStore.getState().status?.simRunning ?? false;
    if (simRunning) {
      set({ pendingInstall: true });
    } else {
      void get().performInstall();
    }
  },

  performInstall: async () => {
    const info = get().info;
    set({ pendingInstall: false });
    try {
      await api.openLocalPath(FSDT_INSTALLER_LNK);
    } catch (e) {
      console.warn("no pude abrir FSDT Installer:", e);
    }
    // El usuario va a actualizar GSX → marcamos la última como instalada y
    // BORRAMOS el aviso (no re-avisar por la misma versión).
    if (info?.latestVersion) {
      const v = info.latestVersion;
      try {
        await api.gsxSetInstalledVersion(v);
      } catch {
        /* ignore */
      }
      // El aviso se borra al poner hasUpdate=false; NO tocamos el flag de
      // "descartado" (ese es sólo para la X) → así el próximo chequeo, ya con
      // installed=última, tampoco avisa, y se puede re-testear sin bloqueos.
      set({ info: { ...info, installedVersion: v, hasUpdate: false } });
    }
  },

  cancelInstall: () => set({ pendingInstall: false }),

  dismiss: () => {
    const v = get().info?.latestVersion ?? null;
    if (!v) return;
    try {
      localStorage.setItem(DISMISSED_KEY, v);
    } catch {
      /* ignore */
    }
    set({ dismissedVersion: v });
  },
}));

/** Helper: ¿hay update de GSX que mostrar (no descartado)? */
export function gsxUpdateVisible(
  info: GsxUpdateInfo | null,
  dismissedVersion: string | null,
): boolean {
  return (
    !!info?.hasUpdate &&
    info.latestVersion != null &&
    info.latestVersion !== dismissedVersion
  );
}
