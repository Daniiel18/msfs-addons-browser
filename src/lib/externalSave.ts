import { create } from "zustand";
import { api } from "./tauri";
import { t } from "./i18n";
import type { DropInspection, DropItem } from "./types";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v8.3) Contenido que NO va a MSFS (apps, instaladores, navdata, cracks…):
 * se pide una carpeta, el backend lo extrae ahí (sin abrir los .zip internos)
 * y se abre en el Explorador. Si la descarga trae notas de instalación
 * (aerohub), se muestran en un aviso; si no, basta un toast.
 */

export const isExternalKind = (kind: string) =>
  kind === "external_content" || kind === "external_rest";

const LAST_FOLDER_KEY = "simfleet.external.lastFolder";

function lastFolder(): string | undefined {
  try {
    return localStorage.getItem(LAST_FOLDER_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function rememberFolder(folder: string) {
  try {
    localStorage.setItem(LAST_FOLDER_KEY, folder);
  } catch {
    /* sin localStorage: el diálogo abre en la carpeta por defecto */
  }
}

export interface SavedInfo {
  name: string;
  path: string;
  isFile: boolean;
  notes: string | null;
}

/** Aviso "Guardado" con las notas de instalación (lo pinta ExternalSavedDialog). */
export const useExternalSavedStore = create<{
  info: SavedInfo | null;
  show: (info: SavedInfo) => void;
  close: () => void;
}>((set) => ({
  info: null,
  show: (info) => set({ info }),
  close: () => set({ info: null }),
}));

/** Nombre de la carpeta a crear: el del addon (descarga del catálogo) o el del
 *  archivo; el "resto" lleva " (extras)". */
export function externalName(insp: DropInspection, item: DropItem): string {
  const fileStem = (insp.archivePath.split(/[\\/]/).pop() ?? "").replace(/\.[^.]+$/, "");
  if (item.kind === "external_rest") {
    return `${insp.displayName ?? fileStem} (${t("external.extras")})`;
  }
  return insp.displayName ?? item.label;
}

export function openSaved(path: string, isFile: boolean) {
  return api.revealPath(path, isFile).catch(() => {});
}

/** Pide la carpeta, guarda y la abre. Devuelve `null` si el usuario cerró el
 *  diálogo sin elegir; lanza si el guardado falla (el que llama avisa). */
export async function saveExternalItem(
  insp: DropInspection,
  item: DropItem,
): Promise<SavedInfo | null> {
  const name = externalName(insp, item);
  const folder = await api.pickFolderPath({
    title: t("external.pick_title", { name }),
    defaultPath: lastFolder(),
  });
  if (!folder) return null;
  rememberFolder(folder);
  const res = await api.dropSaveExternal(insp.sessionId, item.sourcePath, folder, name);
  const info: SavedInfo = { name, path: res.path, isFile: res.isFile, notes: insp.notes ?? null };
  void openSaved(res.path, res.isFile);
  if (info.notes) {
    useExternalSavedStore.getState().show(info);
  } else {
    // Deja claro que NO se instaló nada (al abrirse el Explorador parecía que sí).
    useToastStore.getState().push({
      kind: "info",
      title: t("external.saved", { name }),
      message: t("external.saved_hint"),
      ttlMs: 9000,
      onClick: () => void openSaved(res.path, res.isFile),
    });
  }
  return info;
}

/** (v7) Traduce sentinelas de error del backend a mensajes legibles.
 *  `NO_SPACE:<needed_bytes>:<free_bytes>` → aviso claro con GB (el addon no
 *  cabía al descomprimirse o guardarse). El resto de errores pasan tal cual. */
export function formatDropError(err: string): string {
  const m = err.match(/NO_SPACE:(\d+):(\d+)/);
  if (m) {
    const gb = (b: string) => (Number(b) / 1073741824).toFixed(1);
    const needed = Number(m[1]);
    // `needed=0` → no se pudo estimar el tamaño (archivo bloqueado/cifrado que
    // no lista); mensaje genérico sin cifra falsa. Si sí se estimó, la damos.
    if (needed > 0) {
      return t("drop.no_space", { needed: gb(m[1]), free: gb(m[2]) });
    }
    return t("drop.no_space_generic", { free: gb(m[2]) });
  }
  return err;
}
