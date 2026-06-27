import { create } from "zustand";
import { api } from "../lib/tauri";
import type { GsxUpdateInfo } from "../lib/types";

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
  /** Consulta el backend (llamado al arranque y cada hora). */
  check: () => Promise<void>;
  /** Abre el FSDT Installer y marca la última versión como instalada. */
  openInstaller: () => Promise<void>;
  /** Oculta el aviso para la versión actual (no vuelve hasta una más nueva). */
  dismiss: () => void;
}

export const useGsxUpdateStore = create<GsxUpdateState>((set, get) => ({
  info: null,
  dismissedVersion: loadDismissed(),

  check: async () => {
    try {
      const info = await api.gsxCheckUpdate();
      set({ info });
    } catch (e) {
      console.warn("gsxCheckUpdate falló:", e);
    }
  },

  openInstaller: async () => {
    const info = get().info;
    try {
      await api.openLocalPath(FSDT_INSTALLER_LNK);
    } catch (e) {
      console.warn("no pude abrir FSDT Installer:", e);
    }
    // El usuario va a actualizar GSX → marcamos la última como instalada para
    // no re-avisar por la misma versión.
    if (info?.latestVersion) {
      try {
        await api.gsxSetInstalledVersion(info.latestVersion);
      } catch {
        /* ignore */
      }
      set({
        info: {
          ...info,
          installedVersion: info.latestVersion,
          hasUpdate: false,
        },
      });
    }
  },

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
