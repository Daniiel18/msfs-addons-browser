import { api } from "./tauri";

/**
 * (v8.3) Puente entre un archivo descargado DENTRO de SimFleet (hosters,
 * aerohub, flightsim.to) y su job del panel de Descargas, para contar ahí qué
 * pasa al procesarlo: procesando → listo → instalado / guardado / cancelado.
 * Lo usan el overlay de instalación, el modal y la cola de Notificaciones.
 */

export type DownloadJobStatus =
  | "processing"
  | "ready"
  | "installed"
  | "saved"
  | "failed"
  | "cancelled";

/** Ruta del archivo → clave del job (`hoster-dl-<job>` o id del job). */
const keys = new Map<string, string>();

export function rememberJobKey(path: string, key: string) {
  keys.set(path, key);
}

export function hasJobKey(path: string): boolean {
  return keys.has(path);
}

/** Cuenta el estado al job del archivo (si vino de una descarga interna). */
export function jobStatus(path: string, status: DownloadJobStatus, message?: string) {
  const key = keys.get(path);
  if (!key) return;
  void api.downloadJobStatus(key, status, message).catch(() => {});
  if (status === "installed" || status === "saved" || status === "failed" || status === "cancelled") {
    keys.delete(path);
  }
}

/** ¿El archivo es una descarga interna de SimFleet (carpeta temporal propia)?
 *  Los que arrastra el usuario NUNCA se borran sin preguntar. */
export function isInternalDownload(path: string): boolean {
  const p = path.replace(/\\/g, "/");
  return p.includes("simfleet-livery-dl") || p.includes("simfleet-mirror-dl");
}

/** Instalación cancelada (o imposible) de una descarga interna: el archivo
 *  temporal se BORRA ya (antes esperaba a la purga del siguiente arranque) y
 *  el job queda cancelado / con error en vez de "esperando". */
export async function discardInternalDownload(
  path: string,
  status: "cancelled" | "failed",
  message?: string,
) {
  jobStatus(path, status, message);
  if (isInternalDownload(path)) {
    await api.deleteDroppedArchive(path).catch(() => {});
  }
}
