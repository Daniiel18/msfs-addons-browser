/**
 * (v6 #3) Credenciales por defecto de Live VS (Supabase Realtime).
 *
 * La `anon public` key es pública por diseño (pensada para ir en clientes;
 * protegida por RLS y, como Live VS no usa NINGUNA tabla — solo presence +
 * broadcast de Realtime — no expone datos). Por eso se embebe aquí para que
 * el modo funcione "out of the box" sin que el usuario configure nada.
 *
 * Ajustes → Live VS sigue permitiendo SOBREESCRIBIR estas credenciales (p.ej.
 * para apuntar a otro proyecto); si los campos están vacíos, se usan estas.
 */
export const DEFAULT_VS_SUPABASE_URL = "https://irdsvqxnomatntkwfmax.supabase.co";

export const DEFAULT_VS_SUPABASE_KEY =
  "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImlyZHN2cXhub21hdG50a3dmbWF4Iiwicm9sZSI6ImFub24iLCJpYXQiOjE3ODE5MzY5NTIsImV4cCI6MjA5NzUxMjk1Mn0.WnRtvnrvxoY4v5mDFZWnNeq9iPrmcEK2rRoQN6tW8Nw";

/**
 * Normaliza una Project URL de Supabase: quita el sufijo de endpoint REST
 * (`/rest/v1`) y barras finales, dejando la base que espera supabase-js.
 * Robusto frente a que el usuario pegue la URL del panel "Data API".
 */
export function normalizeSupabaseUrl(raw: string): string {
  return raw
    .trim()
    .replace(/\/rest\/v1\/?$/i, "")
    .replace(/\/+$/, "");
}
