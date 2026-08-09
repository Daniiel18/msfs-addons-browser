import { isTauri } from "./tauri";

/**
 * (v7.4.0) Notificaciones NATIVAS del sistema operativo (el Action Center
 * de Windows), separadas del `<Toaster/>` in-app. Se disparan sobre todo
 * cuando la ventana está en SEGUNDO PLANO: SimFleet suele correr
 * minimizado a la bandeja mientras el usuario vuela, así que un aviso de
 * "descarga terminada" tiene que llegar aunque la app no esté al frente.
 * Cuando la ventana TIENE foco dejamos que el toast in-app haga el
 * trabajo y NO duplicamos (salvo `force`, p. ej. errores).
 *
 * La preferencia vive en localStorage (`sf_native_notifications`, ON por
 * defecto). En un navegador (build demo) todo esto es no-op.
 */

const PREF_KEY = "sf_native_notifications";

/** ¿El usuario tiene activadas las notificaciones nativas? (default ON). */
export function nativeNotificationsEnabled(): boolean {
  if (typeof localStorage === "undefined") return true;
  return localStorage.getItem(PREF_KEY) !== "0";
}

/** Activa/desactiva las notificaciones nativas (persistente). */
export function setNativeNotificationsEnabled(on: boolean): void {
  try {
    localStorage.setItem(PREF_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}

let permissionChecked = false;
let permissionGranted = false;

/** Pide (una vez) el permiso de notificaciones y cachea el resultado. */
async function ensurePermission(): Promise<boolean> {
  if (permissionChecked) return permissionGranted;
  permissionChecked = true;
  try {
    const { isPermissionGranted, requestPermission } = await import(
      "@tauri-apps/plugin-notification"
    );
    permissionGranted = await isPermissionGranted();
    if (!permissionGranted) {
      const res = await requestPermission();
      permissionGranted = res === "granted";
    }
  } catch (e) {
    console.warn("[nativeNotify] permission check failed:", e);
    permissionGranted = false;
  }
  return permissionGranted;
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
  /** Notificar aunque la ventana esté enfocada (p. ej. errores). */
  force?: boolean;
}

/**
 * Dispara una notificación nativa. No hace nada en el navegador (demo),
 * si el usuario las desactivó, o si la ventana está enfocada (salvo
 * `force`). Nunca lanza: cualquier fallo se loguea y se ignora.
 */
export async function nativeNotify(opts: NativeNotifyOptions): Promise<void> {
  if (!isTauri) return;
  if (!nativeNotificationsEnabled()) return;
  try {
    if (!opts.force && (await windowFocused())) return;
    if (!(await ensurePermission())) return;
    const { sendNotification } = await import(
      "@tauri-apps/plugin-notification"
    );
    sendNotification({ title: opts.title, body: opts.body });
  } catch (e) {
    console.warn("[nativeNotify] failed:", e);
  }
}
