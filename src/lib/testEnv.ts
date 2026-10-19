/**
 * (v7.4.2) ENTORNO DE PRUEBAS — lado frontend.
 *
 * El backend (instancia de pruebas, ver `src-tauri/src/test_env.rs`) inyecta
 * `window.__SIMFLEET_TEST_ENV__ = true` en el webview ANTES de cargar la
 * página, así que esto se sabe de forma síncrona desde el primer render — sin
 * esperar un IPC. Importante para Live VS: si arrancara contra el canal real
 * aunque fuera un instante, el rival vería a la instancia de pruebas.
 *
 * En pruebas, todo lo que va a Supabase usa su propio espacio:
 *   · tablas `<nombre>_test` (`vs_results_test`, `pilot_avatars_test`)
 *   · canales Realtime `simfleet-test-vs-…`
 */

declare global {
  interface Window {
    __SIMFLEET_TEST_ENV__?: boolean;
  }
}

export function isTestEnv(): boolean {
  return typeof window !== "undefined" && window.__SIMFLEET_TEST_ENV__ === true;
}

/** Nombre de tabla de Supabase para este entorno. */
export function supabaseTable(name: string): string {
  return isTestEnv() ? `${name}_test` : name;
}

/** Prefijo de los canales Realtime de Live VS para este entorno. */
export function vsChannelPrefix(): string {
  return isTestEnv() ? "simfleet-test-vs-" : "simfleet-vs-";
}
