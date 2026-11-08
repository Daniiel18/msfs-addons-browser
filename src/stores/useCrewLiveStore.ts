import { create } from "zustand";
import { createClient, type RealtimeChannel, type SupabaseClient } from "@supabase/supabase-js";
import { vsChannelPrefix } from "../lib/testEnv";
import type { VsPos } from "./useLiveVsStore";

/**
 * (8.4.x) Presencia EN VIVO siempre activa entre los dos pilotos (Daniel y
 * Héctor) para la pantalla «En vuelo»: cada app anuncia quién es y, cuando
 * vuela, su vuelo (ruta, avión) y su posición — sea cual sea la ruta. El
 * duelo por ruta y día (`useLiveVsStore`) sigue igual; esto sólo añade "ver
 * al otro" siempre que vuele.
 *
 * Canal `<prefijo>presence` (en el entorno de pruebas, `simfleet-test-vs-…`).
 * Mismas credenciales de Supabase que Live VS. Si el otro piloto tiene una
 * versión sin presencia, simplemente no aparece (y el duelo sigue valiendo).
 */

export interface CrewPilot {
  /** "daniel" | "hector" (perfil del piloto). */
  identity: string;
  name: string;
}

/** Vuelo que el otro piloto anuncia (del OFP y del simulador). */
export interface CrewFlight {
  origin: string | null;
  dest: string | null;
  callsign: string | null;
  aircraft: string | null;
  registration: string | null;
  airline: string | null;
}

interface CrewLiveState {
  connected: boolean;
  /** El otro piloto conectado a la presencia (aunque no vuele). */
  friend: CrewPilot | null;
  friendFlight: CrewFlight | null;
  friendPos: VsPos | null;
  /** `Date.now()` de la última posición recibida. */
  friendPosAt: number;
  friendPhase: string | null;
  friendTrack: { lat: number; lon: number }[];
  start: (url: string, key: string, me: CrewPilot) => void;
  /** Emite mi posición y mi vuelo (llamar ~cada pocos segundos mientras vuelo). */
  sendPos: (pos: VsPos, phase: string | null, flight: CrewFlight | null) => void;
  stop: () => void;
}

/** Sin posición en este tiempo, el otro piloto ya no está volando. */
export const CREW_STALE_MS = 20_000;

let client: SupabaseClient | null = null;
let channel: RealtimeChannel | null = null;
let channelKey = "";
let me: CrewPilot | null = null;
/** Último vuelo anunciado por presencia (para no re-anunciar en cada tick). */
let trackedSig = "";

const sameRoute = (a: CrewFlight | null, b: CrewFlight | null) =>
  (a?.origin ?? "") === (b?.origin ?? "") && (a?.dest ?? "") === (b?.dest ?? "");

export const useCrewLiveStore = create<CrewLiveState>((set, get) => ({
  connected: false,
  friend: null,
  friendFlight: null,
  friendPos: null,
  friendPosAt: 0,
  friendPhase: null,
  friendTrack: [],

  start: (url, key, pilot) => {
    const name = `${vsChannelPrefix()}presence`;
    const sig = `${url}|${key}|${pilot.identity}|${name}`;
    if (channel && channelKey === sig) return;
    get().stop();
    try {
      client = createClient(url, key, { realtime: { params: { eventsPerSecond: 5 } } });
    } catch {
      return;
    }
    me = pilot;
    channelKey = sig;
    const ch = client.channel(name, {
      config: { presence: { key: pilot.identity }, broadcast: { self: false } },
    });

    const syncFriend = () => {
      const state = ch.presenceState() as Record<string, Array<Record<string, unknown>>>;
      let friend: CrewPilot | null = null;
      let flight: CrewFlight | null = null;
      for (const metas of Object.values(state)) {
        const m = metas[0] as { identity?: string; name?: string; flight?: CrewFlight | null } | undefined;
        if (m?.identity && m.identity !== pilot.identity) {
          friend = { identity: m.identity, name: m.name ?? m.identity };
          flight = m.flight ?? null;
          break;
        }
      }
      const patch: Partial<CrewLiveState> = { friend };
      if (!friend) {
        Object.assign(patch, { friendFlight: null, friendPos: null, friendPosAt: 0, friendPhase: null, friendTrack: [] });
      } else if (flight || !get().friendPos) {
        patch.friendFlight = flight;
      }
      set(patch);
    };
    ch.on("presence", { event: "sync" }, syncFriend);
    ch.on("presence", { event: "join" }, syncFriend);
    ch.on("presence", { event: "leave" }, syncFriend);
    ch.on("broadcast", { event: "pos" }, ({ payload }) => {
      const p = payload as
        | (VsPos & { identity?: string; name?: string; phase?: string | null; flight?: CrewFlight | null })
        | undefined;
      if (!p || !p.identity || p.identity === pilot.identity) return;
      const st = get();
      const flight = p.flight ?? st.friendFlight;
      // Traza: se reinicia al cambiar de ruta o si la posición salta (otro vuelo).
      const prev = sameRoute(flight, st.friendFlight) ? st.friendTrack : [];
      const last = prev[prev.length - 1];
      let track = prev;
      if (!last || Math.abs(last.lat - p.lat) >= 1e-6 || Math.abs(last.lon - p.lon) >= 1e-6) {
        track =
          last && (Math.abs(last.lat - p.lat) > 1.5 || Math.abs(last.lon - p.lon) > 1.5)
            ? [{ lat: p.lat, lon: p.lon }]
            : [...prev, { lat: p.lat, lon: p.lon }].slice(-3000);
      }
      set({
        friend: st.friend ?? { identity: p.identity, name: p.name ?? p.identity },
        friendFlight: flight,
        friendPos: { lat: p.lat, lon: p.lon, heading: p.heading, alt: p.alt, gs: p.gs },
        friendPosAt: Date.now(),
        friendPhase: p.phase ?? null,
        friendTrack: track,
      });
    });
    ch.subscribe((status) => {
      if (status === "SUBSCRIBED") {
        set({ connected: true });
        trackedSig = "";
        void ch.track({ identity: pilot.identity, name: pilot.name, flight: null });
      } else if (status === "CLOSED" || status === "CHANNEL_ERROR") {
        set({ connected: false });
      }
    });
    channel = ch;
  },

  sendPos: (pos, phase, flight) => {
    if (!channel || !get().connected || !me) return;
    // La presencia lleva el vuelo actual (para quien entre después); sólo se
    // re-anuncia cuando cambia.
    const sig = JSON.stringify(flight);
    if (sig !== trackedSig) {
      trackedSig = sig;
      void channel.track({ identity: me.identity, name: me.name, flight });
    }
    void channel.send({
      type: "broadcast",
      event: "pos",
      payload: { identity: me.identity, name: me.name, phase, flight, ...pos },
    });
  },

  stop: () => {
    if (channel && client) {
      try {
        void client.removeChannel(channel);
      } catch {
        /* ignore */
      }
    }
    channel = null;
    channelKey = "";
    me = null;
    trackedSig = "";
    set({
      connected: false,
      friend: null,
      friendFlight: null,
      friendPos: null,
      friendPosAt: 0,
      friendPhase: null,
      friendTrack: [],
    });
  },
}));

/** ¿El otro piloto está volando ahora (posición reciente)? */
export function friendFlying(posAt: number, now = Date.now()): boolean {
  return posAt > 0 && now - posAt < CREW_STALE_MS;
}
