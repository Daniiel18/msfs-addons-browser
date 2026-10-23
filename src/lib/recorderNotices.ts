import { listen } from "@tauri-apps/api/event";
import { isTauri } from "./tauri";
import { t } from "./i18n";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v8.3.2) Avisos del grabador de aterrizajes (`recorder://notice`).
 *
 * El usuario eligió enterarse de los fallos SÓLO dentro de SimFleet: toast
 * persistente (se queda hasta cerrarlo), porque cuando pasa está volando y
 * lo verá al volver a la app.
 *   · "error"  — la captura no pudo grabar (carpeta, codificador…).
 *   · "folder" — la carpeta elegida no estaba: se grabó en Vídeos.
 */
interface RecorderNotice {
  kind: "error" | "folder";
  message: string;
}

let wired = false;

export function initRecorderNotices() {
  if (wired || !isTauri) return;
  wired = true;
  void listen<RecorderNotice>("recorder://notice", (e) => {
    const n = e.payload;
    if (!n?.message) return;
    useToastStore.getState().push({
      kind: n.kind === "error" ? "error" : "info",
      title: t(n.kind === "error" ? "rec.notice.error_title" : "rec.notice.folder_title"),
      message: n.message,
      ttlMs: null,
    });
  });
}
