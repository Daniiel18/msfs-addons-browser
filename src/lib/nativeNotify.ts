import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./tauri";

/**
 * (v7.4.0) Avisos de ESCRITORIO, separados del `<Toaster/>` in-app. Se
 * disparan sobre todo cuando la ventana está en SEGUNDO PLANO: SimFleet suele
 * correr minimizado a la bandeja mientras el usuario vuela, así que un aviso
 * de "descarga terminada" tiene que llegar aunque la app no esté al frente.
 * Cuando la ventana TIENE foco dejamos que el toast in-app haga el trabajo y
 * NO duplicamos (salvo `force`, p. ej. errores).
 *
 * (23/10) Ya no son notificaciones de Windows (diseño de Windows y, en el
 * entorno de pruebas, firmadas por "PowerShell"): son una ventana propia con
 * el diseño de SimFleet en la esquina de la pantalla, encima del sim y sin
 * robarle el foco (`desktop_notice` en el backend).
 *
 * La preferencia vive en localStorage (`sf_native_notifications`, ON por
 * defecto). En un navegador (build demo) todo esto es no-op.
 */

const PREF_KEY = "sf_native_notifications";

/** ¿El usuario tiene activados los avisos de escritorio? (default ON). */
export function nativeNotificationsEnabled(): boolean {
  if (typeof localStorage === "undefined") return true;
  return localStorage.getItem(PREF_KEY) !== "0";
}

/** Activa/desactiva los avisos de escritorio (persistente). */
export function setNativeNotificationsEnabled(on: boolean): void {
  try {
    localStorage.setItem(PREF_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}

/** ¿La ventana principal está enfocada ahora mismo? */
async function windowFocused(): Promise<boolean> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().isFocused();
  } catch {
    // Si no lo podemos saber, asumimos NO enfocada para no tragarnos el aviso.
    return false;
  }
}

export interface NativeNotifyOptions {
  title: string;
  body?: string;
  /** Color del aviso (por defecto "info"). */
  kind?: "info" | "success" | "error";
  /** Notificar aunque la ventana esté enfocada (p. ej. errores). */
  force?: boolean;
}

/**
 * Muestra un aviso de escritorio. No hace nada en el navegador (demo), si el
 * usuario los desactivó, o si la ventana está enfocada (salvo `force`).
 * Nunca lanza: cualquier fallo se loguea y se ignora.
 */
export async function nativeNotify(opts: NativeNotifyOptions): Promise<void> {
  if (!isTauri) return;
  if (!nativeNotificationsEnabled()) return;
  try {
    if (!opts.force && (await windowFocused())) return;
    await invoke("desktop_notice", {
      title: opts.title,
      body: opts.body ?? null,
      kind: opts.kind ?? "info",
    });
  } catch (e) {
    console.warn("[nativeNotify] failed:", e);
  }
}
