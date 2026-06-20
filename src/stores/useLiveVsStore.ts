import { create } from "zustand";
import {
  createClient,
  type SupabaseClient,
  type RealtimeChannel,
} from "@supabase/supabase-js";

/**
 * (v6 #3) Live VS — estado del modo competitivo Daniel vs Héctor sobre
 * Supabase Realtime.
 *
 * Modelo: ambos pilotos, volando la MISMA ruta el MISMO día, se unen al mismo
 * canal `simfleet-vs-<fecha>-<origen>-<destino>`. Por **presence** cada cliente
 * publica su identidad + OFP; cuando aparece el rival, el botón "Crew VS" se
 * habilita. Por **broadcast** cada cliente emite su lat/lon en vivo y el otro
 * dibuja el avión rival en el routemap existente (no se crea un mapa nuevo).
 *
 * El cliente Supabase corre en el webview; las credenciales salen de Ajustes
 * (no hay nada en código). Si no hay credenciales, todo queda inerte.
 */

export interface VsOfp {
  origin: string;
  dest: string;
  date: string; // YYYY-MM-DD
  aircraft: string | null;
  registration: string | null;
  callsign: string | null;
  flightNumber: string | null;
}

export interface VsPilot {
  identity: string; // "daniel" | "hector"
  name: string;
  ofp: VsOfp;
}

export interface VsPos {
  lat: number;
  lon: number;
  heading: number;
  alt: number;
  gs: number;
}

interface LiveVsState {
  connected: boolean;
  self: VsPilot | null;
  rival: VsPilot | null;
  rivalPos: VsPos | null;
  /** Hay rival presente en el mismo canal (misma ruta+día). */
  matchReady: boolean;
  /** Última actualización de la posición rival (ms epoch) — para "stale". */
  rivalPosAt: number;

  start: (url: string, key: string, self: VsPilot) => void;
  broadcastPos: (pos: VsPos) => void;
  stop: () => void;
}

// El cliente/canal NO van en el estado (no serializables) — refs de módulo.
let client: SupabaseClient | null = null;
let channel: RealtimeChannel | null = null;
let currentChannelName = "";

function channelName(ofp: VsOfp): string {
  return `simfleet-vs-${ofp.date}-${ofp.origin}-${ofp.dest}`.toLowerCase();
}

export const useLiveVsStore = create<LiveVsState>((set, get) => ({
  connected: false,
  self: null,
  rival: null,
  rivalPos: null,
  matchReady: false,
  rivalPosAt: 0,

  start: (url, key, self) => {
    const name = channelName(self.ofp);
    // Ya conectados al mismo canal con la misma identidad → nada que hacer.
    if (channel && currentChannelName === name && get().self?.identity === self.identity) {
      set({ self });
      return;
    }
    get().stop();

    try {
      client = createClient(url, key, {
        realtime: { params: { eventsPerSecond: 5 } },
      });
    } catch {
      return;
    }

    currentChannelName = name;
    set({ self, connected: false, rival: null, rivalPos: null, matchReady: false });

    const ch = client.channel(name, {
      config: {
        presence: { key: self.identity },
        broadcast: { self: false },
      },
    });

    const syncRival = () => {
      const stateMap = ch.presenceState() as Record<string, Array<Record<string, unknown>>>;
      let rival: VsPilot | null = null;
      for (const metas of Object.values(stateMap)) {
        const m = metas[0] as unknown as VsPilot | undefined;
        if (m && m.identity && m.identity !== self.identity) {
          rival = m;
          break;
        }
      }
      set({ rival, matchReady: rival != null });
    };

    ch.on("presence", { event: "sync" }, syncRival);
    ch.on("presence", { event: "join" }, syncRival);
    ch.on("presence", { event: "leave" }, syncRival);
    ch.on("broadcast", { event: "pos" }, ({ payload }) => {
      const p = payload as (VsPos & { identity?: string }) | undefined;
      if (!p || p.identity === self.identity) return;
      set({
        rivalPos: { lat: p.lat, lon: p.lon, heading: p.heading, alt: p.alt, gs: p.gs },
        rivalPosAt: Date.now(),
      });
    });

    ch.subscribe((status) => {
      if (status === "SUBSCRIBED") {
        set({ connected: true });
        void ch.track({
          identity: self.identity,
          name: self.name,
          ofp: self.ofp,
        });
      } else if (status === "CLOSED" || status === "CHANNEL_ERROR") {
        set({ connected: false });
      }
    });

    channel = ch;
  },

  broadcastPos: (pos) => {
    if (!channel || !get().connected) return;
    const self = get().self;
    void channel.send({
      type: "broadcast",
      event: "pos",
      payload: { identity: self?.identity, ...pos },
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
    currentChannelName = "";
    set({
      connected: false,
      rival: null,
      rivalPos: null,
      matchReady: false,
    });
  },
}));
