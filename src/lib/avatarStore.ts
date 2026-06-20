import { createClient, type SupabaseClient } from "@supabase/supabase-js";
import { useSettingsStore } from "../stores/useSettingsStore";
import {
  DEFAULT_VS_SUPABASE_URL,
  DEFAULT_VS_SUPABASE_KEY,
  normalizeSupabaseUrl,
} from "./liveVsConfig";

/**
 * (v6.1 #30) Persistencia del avatar del piloto: LOCAL (localStorage) + NUBE
 * (Supabase, SOLO la foto). Por orden del usuario activamos la nube únicamente
 * para el avatar — NO se suben puntos ni vuelos (el sync general sigue inactivo).
 *
 * Tabla esperada en Supabase (créala una vez en el SQL editor):
 *
 *   create table if not exists pilot_avatars (
 *     identity text primary key,
 *     data_url text not null,
 *     updated_at timestamptz default now()
 *   );
 *   alter table pilot_avatars enable row level security;
 *   create policy "anon all" on pilot_avatars for all using (true) with check (true);
 *
 * Si la tabla no existe, todo degrada a SOLO local sin romper nada.
 */

const localKey = (identity: string) => `simfleet:pilot-avatar:${identity}`;
const LEGACY_KEY = "simfleet:pilot-avatar"; // clave única antigua (sin identidad)

// Listeners por identidad para que el header (y cualquier vista) refresquen
// cuando el avatar cambia (subida local o llegada desde la nube).
const listeners = new Map<string, Set<(url: string | null) => void>>();

function emit(identity: string, url: string | null) {
  listeners.get(identity)?.forEach((cb) => cb(url));
}

export function subscribeAvatar(
  identity: string,
  cb: (url: string | null) => void,
): () => void {
  let set = listeners.get(identity);
  if (!set) {
    set = new Set();
    listeners.set(identity, set);
  }
  set.add(cb);
  return () => {
    set?.delete(cb);
  };
}

function readLocal(identity: string): string | null {
  try {
    return (
      localStorage.getItem(localKey(identity)) ??
      localStorage.getItem(LEGACY_KEY)
    );
  } catch {
    return null;
  }
}

function writeLocal(identity: string, url: string) {
  try {
    localStorage.setItem(localKey(identity), url);
  } catch {
    /* localStorage lleno/deshabilitado — la nube sigue como respaldo */
  }
}

// Cliente Supabase compartido (solo para el avatar). Credenciales de Ajustes
// si las hay, si no las embebidas (mismas que Live VS).
let client: SupabaseClient | null = null;
function getClient(): SupabaseClient | null {
  if (client) return client;
  try {
    const s = useSettingsStore.getState().settings;
    const url = normalizeSupabaseUrl(
      (s.vsSupabaseUrl ?? "").trim() || DEFAULT_VS_SUPABASE_URL,
    );
    const key = (s.vsSupabaseKey ?? "").trim() || DEFAULT_VS_SUPABASE_KEY;
    if (!url || !key) return null;
    client = createClient(url, key);
    return client;
  } catch {
    return null;
  }
}

/**
 * Carga el avatar: devuelve YA el valor local (sin parpadeo) y, en paralelo,
 * consulta la nube; si la nube trae algo distinto, actualiza local + notifica.
 */
export async function loadAvatar(identity: string): Promise<string | null> {
  const local = readLocal(identity);
  // Fetch nube en segundo plano (no bloquea el retorno local).
  void (async () => {
    const c = getClient();
    if (!c) return;
    try {
      const { data, error } = await c
        .from("pilot_avatars")
        .select("data_url")
        .eq("identity", identity)
        .maybeSingle();
      if (error || !data?.data_url) return;
      if (data.data_url !== local) {
        writeLocal(identity, data.data_url);
        emit(identity, data.data_url);
      }
    } catch {
      /* tabla inexistente / offline → solo local */
    }
  })();
  return local;
}

/** Guarda el avatar local + sube a la nube (solo la foto). */
export async function saveAvatar(identity: string, dataUrl: string): Promise<void> {
  writeLocal(identity, dataUrl);
  emit(identity, dataUrl);
  const c = getClient();
  if (!c) return;
  try {
    await c
      .from("pilot_avatars")
      .upsert({ identity, data_url: dataUrl, updated_at: new Date().toISOString() });
  } catch {
    /* sin tabla/conexión → queda solo local, se reintenta en el próximo guardado */
  }
}
