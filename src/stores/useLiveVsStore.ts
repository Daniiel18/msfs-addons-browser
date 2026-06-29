import { create } from "zustand";
import {
  createClient,
  type SupabaseClient,
  type RealtimeChannel,
} from "@supabase/supabase-js";
import { useToastStore } from "./useToastStore";

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

/** (v6.1 #34) Resultado de aterrizaje para la comparativa final del duelo. */
export interface VsLanding {
  fpm: number;
  grade: string; // "butter" | "acceptable" | "hard"
}

/** (v6.2.2) Avión REAL que cada piloto está volando en el sim (no el del OFP de
 *  SimBrief, que puede no coincidir). Se difunde en el broadcast de posición
 *  para que la ficha muestre la foto/matrícula correctas. */
export interface VsAircraft {
  registration: string | null; // matrícula real del sim (para la foto)
  aircraftType: string | null; // tipo (ej. "B38M")
  airlineName: string | null; // aerolínea real (nombre o ICAO, para el logo)
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
  /** (v6.1 #34) Aterrizaje propio / del rival (para el resultado del duelo). */
  selfLanding: VsLanding | null;
  rivalLanding: VsLanding | null;
  /** (v6.2.2) Avión REAL en vuelo de cada piloto (del sim, no del OFP). */
  selfAircraft: VsAircraft | null;
  rivalAircraft: VsAircraft | null;
  /** (v6.2.7) Fase del rival (climbing/cruise/…), difundida en el broadcast.
   *  La usa el panel "Flying now" del rival al hacer clic en su avión. */
  rivalPhase: string | null;
  /** (v6.2.7) Canal del duelo EN VIVO actual (vacío si no hay). El FlightBook lo
   *  compara con el del vuelo seleccionado para decidir live vs histórico. */
  currentChannel: string;
  /** (v6.2.11) Avisos de "Crew VS listo" (duelos completados, ambos aterrizaron)
   *  pendientes de ver — los muestra la campanita. */
  duelNotices: VsDuelNotice[];

  start: (url: string, key: string, self: VsPilot) => void;
  broadcastPos: (pos: VsPos, phase?: string | null) => void;
  broadcastLanding: (result: VsLanding) => void;
  /** (v6.2.2) Fija el avión real propio (lo difunde el siguiente broadcastPos). */
  setSelfAircraft: (a: VsAircraft) => void;
  /** (v6.2.11) Descarta un aviso de "Crew VS listo" de la campanita. */
  dismissDuelNotice: (channel: string) => void;
  stop: () => void;
}

// El cliente/canal NO van en el estado (no serializables) — refs de módulo.
let client: SupabaseClient | null = null;
let channel: RealtimeChannel | null = null;
let currentChannelName = "";

function channelName(ofp: VsOfp): string {
  return `simfleet-vs-${ofp.date}-${ofp.origin}-${ofp.dest}`.toLowerCase();
}

/**
 * (v6.2.2) Persistencia POR-VUELO del resultado del duelo. El modal vivía sólo
 * en memoria, así que el FPM y el avión "no persistían" (#1) y cada vuelo no
 * guardaba lo suyo (#4). Guardamos en localStorage con clave = nombre de canal
 * (fecha+origen+destino), de modo que cada vuelo tiene su propio registro
 * aislado y sobrevive a remount/cierre de la app. Lo transitorio (posición en
 * vivo) NO se persiste.
 */
interface VsPersisted {
  selfLanding: VsLanding | null;
  rivalLanding: VsLanding | null;
  selfAircraft: VsAircraft | null;
  rivalAircraft: VsAircraft | null;
  // (v6.2.7) Pilotos del duelo — guardados para poder REVISAR el Crew VS de un
  // vuelo PASADO (al seleccionarlo en el FlightBook) aunque el rival no esté
  // online ahora.
  self?: VsPilot | null;
  rival?: VsPilot | null;
}

const VS_STORE_PREFIX = "simfleet.vs.";

function loadPersisted(name: string): VsPersisted | null {
  try {
    const raw = localStorage.getItem(VS_STORE_PREFIX + name);
    return raw ? (JSON.parse(raw) as VsPersisted) : null;
  } catch {
    return null;
  }
}

/** (v6.2.7) Nombre de canal/clave de duelo a partir de fecha+ruta. El FlightBook
 *  lo usa para cargar el Crew VS del vuelo SELECCIONADO. Igual formato que
 *  `channelName(ofp)`. */
export function vsChannelKey(
  date: string,
  origin: string,
  dest: string,
): string {
  return `simfleet-vs-${date}-${origin}-${dest}`.toLowerCase();
}

/** (v6.2.7) Carga el snapshot persistido (duelo) de un canal/clave. */
export function loadVsSnapshot(channel: string): VsPersisted | null {
  return loadPersisted(channel);
}

/** (v6.2.13) Carga el snapshot del duelo de un vuelo TOLERANTE a la fecha: prueba
 *  el día exacto y ±1 día. El canal en vivo se nombra con la fecha de calendario
 *  (UTC) del día del vuelo, y el histórico con `startedAt`; cerca de medianoche /
 *  husos horarios pueden diferir por un día. Devuelve el primer snapshot con
 *  datos (self o rival). */
export function findVsSnapshot(
  dateYmd: string,
  origin: string,
  dest: string,
): VsPersisted | null {
  const base = dateYmd.slice(0, 10);
  const candidates = [base];
  const ms = Date.parse(base + "T00:00:00Z");
  if (!Number.isNaN(ms)) {
    candidates.push(new Date(ms - 86400000).toISOString().slice(0, 10));
    candidates.push(new Date(ms + 86400000).toISOString().slice(0, 10));
  }
  for (const d of candidates) {
    const snap = loadPersisted(vsChannelKey(d, origin, dest));
    if (snap && (snap.self != null || snap.rival != null)) return snap;
  }
  return null;
}

/** (v6.2.13) Regla de propiedad de callsign para la cuenta SimBrief COMPARTIDA:
 *  los callsign de Héctor contienen "357"; todo lo demás es de Daniel. Cada PC
 *  usa esto para elegir SU propio OFP de la lista (y no el del otro) y así no
 *  cruzar números de vuelo entre pilotos. Si las reglas cambian, se edita aquí. */
const HECTOR_CALLSIGN_PATTERN = /357/;
export function vsCallsignOwner(
  callsign: string | null | undefined,
): "hector" | "daniel" {
  return callsign && HECTOR_CALLSIGN_PATTERN.test(callsign) ? "hector" : "daniel";
}

/** (v6.2.11) Aviso de "Crew VS listo" — se genera al COMPLETARSE un duelo
 *  (ambos pilotos aterrizaron). Se muestra en la campanita + toast y persiste
 *  hasta que el usuario lo descarta. */
export interface VsDuelNotice {
  channel: string;
  origin: string;
  dest: string;
  selfName: string;
  rivalName: string;
  selfFpm: number;
  rivalFpm: number;
  at: number;
}

const VS_NOTICES_KEY = "simfleet.vs.notices.v1";

function loadDuelNotices(): VsDuelNotice[] {
  try {
    const raw = localStorage.getItem(VS_NOTICES_KEY);
    return raw ? (JSON.parse(raw) as VsDuelNotice[]) : [];
  } catch {
    return [];
  }
}

function saveDuelNotices(list: VsDuelNotice[]) {
  try {
    localStorage.setItem(VS_NOTICES_KEY, JSON.stringify(list));
  } catch {
    /* ignore */
  }
}

function savePersisted(name: string, data: VsPersisted) {
  try {
    localStorage.setItem(VS_STORE_PREFIX + name, JSON.stringify(data));
  } catch {
    /* localStorage lleno o no disponible — el duelo sigue en memoria */
  }
}

export const useLiveVsStore = create<LiveVsState>((set, get) => {
  // (v6.2.2) Vuelca el resultado del duelo actual a localStorage bajo la clave
  // del canal en curso. Se llama tras cada cambio "duradero" (aterrizaje /
  // avión), no en cada tick de posición.
  const persist = () => {
    if (!currentChannelName) return;
    const st = get();
    savePersisted(currentChannelName, {
      selfLanding: st.selfLanding,
      rivalLanding: st.rivalLanding,
      selfAircraft: st.selfAircraft,
      rivalAircraft: st.rivalAircraft,
      self: st.self,
      rival: st.rival,
    });
  };

  // (v6.2.11) Cuando AMBOS pilotos han aterrizado (duelo completo) y hay rival
  // real, registra un aviso "Crew VS listo" (campanita + toast). Dedup por canal.
  const recordDuelIfComplete = () => {
    const st = get();
    if (
      !currentChannelName ||
      !st.self ||
      !st.rival ||
      !st.selfLanding ||
      !st.rivalLanding
    ) {
      return;
    }
    if (st.duelNotices.some((d) => d.channel === currentChannelName)) return;
    const notice: VsDuelNotice = {
      channel: currentChannelName,
      origin: st.self.ofp.origin,
      dest: st.self.ofp.dest,
      selfName: st.self.name,
      rivalName: st.rival.name,
      selfFpm: st.selfLanding.fpm,
      rivalFpm: st.rivalLanding.fpm,
      at: Date.now(),
    };
    const next = [notice, ...st.duelNotices].slice(0, 30);
    set({ duelNotices: next });
    saveDuelNotices(next);
    try {
      useToastStore.getState().push({
        kind: "success",
        title: "Crew VS listo",
        message: `${notice.origin}→${notice.dest}: tú ${Math.round(
          notice.selfFpm,
        )} vs ${notice.rivalName} ${Math.round(notice.rivalFpm)} fpm`,
        ttlMs: 8000,
      });
    } catch {
      /* ignore */
    }
  };

  // (v6.2.3) Persistencia COMPARTIDA en Supabase (tabla `vs_results`) para que el
  // resultado del rival aparezca AUNQUE no coincidan online (#2/#3). El realtime
  // sólo cubre "ambos conectados a la vez"; esto cubre "uno voló antes / tuvo que
  // salir y volvió luego". Falla en silencio si la tabla no existe → la app sigue
  // con realtime + localStorage. (SQL de creación en las notas del release.)
  const upsertResult = () => {
    const st = get();
    const self = st.self;
    if (!client || !currentChannelName || !self) return;
    void client
      .from("vs_results")
      .upsert(
        {
          channel: currentChannelName,
          identity: self.identity,
          name: self.name,
          callsign: self.ofp.callsign,
          fpm: st.selfLanding?.fpm ?? null,
          grade: st.selfLanding?.grade ?? null,
          registration: st.selfAircraft?.registration ?? null,
          aircraft_type: st.selfAircraft?.aircraftType ?? null,
          airline: st.selfAircraft?.airlineName ?? null,
          updated_at: new Date().toISOString(),
        },
        { onConflict: "channel,identity" },
      )
      .then(
        () => {},
        () => {},
      ); // ignora errores (tabla ausente / RLS / red)
  };

  // Lee los resultados persistidos de ESTE vuelo y los fusiona. Si el rival no
  // llegó por presence (offline), lo RECONSTRUYE desde su fila (misma ruta/día
  // que el propio OFP) para que el botón/modal Crew VS salgan igual.
  const fetchPersistedResults = (name: string, selfPilot: VsPilot) => {
    if (!client) return;
    void client
      .from("vs_results")
      .select("*")
      .eq("channel", name)
      .then(
        ({ data }: { data: Array<Record<string, unknown>> | null }) => {
          if (!data || currentChannelName !== name) return;
          const patch: Partial<LiveVsState> = {};
          for (const row of data) {
            const fpm = (row.fpm as number | null) ?? null;
            const landing: VsLanding | null =
              fpm != null
                ? { fpm, grade: (row.grade as string) ?? "acceptable" }
                : null;
            const reg = (row.registration as string | null) ?? null;
            const acType = (row.aircraft_type as string | null) ?? null;
            const airline = (row.airline as string | null) ?? null;
            const aircraft: VsAircraft | null =
              reg || acType || airline
                ? { registration: reg, aircraftType: acType, airlineName: airline }
                : null;
            if (row.identity === selfPilot.identity) {
              if (landing && !get().selfLanding) patch.selfLanding = landing;
              if (aircraft && !get().selfAircraft) patch.selfAircraft = aircraft;
            } else {
              if (landing) patch.rivalLanding = landing;
              if (aircraft) patch.rivalAircraft = aircraft;
              if (!get().rival) {
                patch.rival = {
                  identity: row.identity as string,
                  name: (row.name as string) ?? (row.identity as string),
                  ofp: {
                    ...selfPilot.ofp,
                    registration: reg,
                    aircraft: acType ?? selfPilot.ofp.aircraft,
                    callsign: (row.callsign as string | null) ?? null,
                    flightNumber: null,
                  },
                };
                patch.matchReady = true;
              }
            }
          }
          if (Object.keys(patch).length) {
            set(patch);
            persist();
          }
        },
        () => {}, // ignora errores
      );
  };

  return {
  connected: false,
  self: null,
  rival: null,
  rivalPos: null,
  matchReady: false,
  rivalPosAt: 0,
  selfLanding: null,
  rivalLanding: null,
  selfAircraft: null,
  rivalAircraft: null,
  rivalPhase: null,
  currentChannel: "",
  duelNotices: loadDuelNotices(),

  start: (url, key, self) => {
    const name = channelName(self.ofp);
    // Ya conectados al mismo canal con la misma identidad → nada que hacer.
    if (channel && currentChannelName === name && get().self?.identity === self.identity) {
      set({ self, currentChannel: name });
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
    // (v6.2.2) Rehidrata el resultado guardado de ESTE vuelo (si lo hubo) para
    // que el FPM y el avión persistan tras remount/reinicio. Si es un vuelo
    // nuevo (otra ruta/día) no habrá registro → pizarra limpia (#4).
    const saved = loadPersisted(name);
    set({
      self,
      currentChannel: name,
      connected: false,
      rival: null,
      rivalPos: null,
      matchReady: false,
      selfLanding: saved?.selfLanding ?? null,
      rivalLanding: saved?.rivalLanding ?? null,
      selfAircraft: saved?.selfAircraft ?? null,
      rivalAircraft: saved?.rivalAircraft ?? null,
      rivalPhase: null,
    });

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
      const p = payload as
        | (VsPos & {
            identity?: string;
            aircraft?: VsAircraft | null;
            phase?: string | null;
          })
        | undefined;
      if (!p || p.identity === self.identity) return;
      // (v6.2.2) El avión del rival sólo cambia muy de vez en cuando — sólo
      // actualizamos (y persistimos) cuando difiere, no en cada tick.
      const prevAc = get().rivalAircraft;
      const acChanged =
        !!p.aircraft &&
        (!prevAc ||
          prevAc.registration !== p.aircraft.registration ||
          prevAc.aircraftType !== p.aircraft.aircraftType ||
          prevAc.airlineName !== p.aircraft.airlineName);
      set({
        rivalPos: { lat: p.lat, lon: p.lon, heading: p.heading, alt: p.alt, gs: p.gs },
        rivalPosAt: Date.now(),
        rivalPhase: p.phase ?? null,
        ...(acChanged ? { rivalAircraft: p.aircraft } : {}),
      });
      if (acChanged) persist();
    });
    // (v6.1 #34) Resultado de aterrizaje del rival.
    ch.on("broadcast", { event: "landing" }, ({ payload }) => {
      const p = payload as (VsLanding & { identity?: string }) | undefined;
      if (!p || p.identity === self.identity) return;
      set({ rivalLanding: { fpm: p.fpm, grade: p.grade } });
      persist(); // (v6.2.2) guarda el aterrizaje del rival para este vuelo
      recordDuelIfComplete(); // (v6.2.11) aviso si el duelo ya está completo
    });

    ch.subscribe((status) => {
      if (status === "SUBSCRIBED") {
        set({ connected: true });
        void ch.track({
          identity: self.identity,
          name: self.name,
          ofp: self.ofp,
        });
        // (v6.2.3) Refresca por si el rival voló mientras tanto.
        fetchPersistedResults(name, self);
      } else if (status === "CLOSED" || status === "CHANNEL_ERROR") {
        set({ connected: false });
      }
    });

    channel = ch;
    // (v6.2.3) Carga inmediata de resultados persistidos (rival offline incluido).
    fetchPersistedResults(name, self);
  },

  broadcastPos: (pos, phase) => {
    if (!channel || !get().connected) return;
    const self = get().self;
    void channel.send({
      type: "broadcast",
      event: "pos",
      // (v6.2.2) Adjuntamos el avión REAL para que el rival muestre la foto
      // correcta. (v6.2.7) + la fase, para el panel "Flying now" del rival.
      payload: {
        identity: self?.identity,
        aircraft: get().selfAircraft,
        phase: phase ?? null,
        ...pos,
      },
    });
  },

  setSelfAircraft: (a) => {
    const cur = get().selfAircraft;
    if (
      cur &&
      cur.registration === a.registration &&
      cur.aircraftType === a.aircraftType &&
      cur.airlineName === a.airlineName
    ) {
      return; // sin cambios
    }
    set({ selfAircraft: a });
    persist(); // (v6.2.2) guarda el avión real propio para este vuelo
    upsertResult(); // (v6.2.3) comparte el avión real con el rival (async)
  },

  dismissDuelNotice: (channel) => {
    const next = get().duelNotices.filter((d) => d.channel !== channel);
    set({ duelNotices: next });
    saveDuelNotices(next);
  },

  broadcastLanding: (result) => {
    // Fija el aterrizaje propio (aunque no haya canal) y lo emite al rival.
    set({ selfLanding: result });
    persist(); // (v6.2.2) persiste el FPM propio para este vuelo (#1)
    upsertResult(); // (v6.2.3) comparte el FPM con el rival aunque esté offline
    recordDuelIfComplete(); // (v6.2.11) aviso si el duelo ya está completo
    if (!channel || !get().connected) return;
    const self = get().self;
    void channel.send({
      type: "broadcast",
      event: "landing",
      payload: { identity: self?.identity, ...result },
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
      currentChannel: "",
      rival: null,
      rivalPos: null,
      matchReady: false,
      selfLanding: null,
      rivalLanding: null,
      rivalAircraft: null,
      rivalPhase: null,
    });
  },
  };
});
