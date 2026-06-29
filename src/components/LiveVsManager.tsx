import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri } from "../lib/tauri";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import {
  useLiveVsStore,
  vsCallsignOwner,
  type VsOfp,
} from "../stores/useLiveVsStore";
import {
  DEFAULT_VS_SUPABASE_URL,
  DEFAULT_VS_SUPABASE_KEY,
  normalizeSupabaseUrl,
} from "../lib/liveVsConfig";

/**
 * (v6 #3) Componente headless que orquesta Live VS: arma el canal Supabase con
 * la identidad del piloto + su OFP actual, y emite la posición en vivo. No
 * pinta nada (el botón/modal/marker viven en sus componentes). Se monta una
 * vez en App.
 */
export function LiveVsManager() {
  const url = useSettingsStore((s) => s.settings.vsSupabaseUrl);
  const key = useSettingsStore((s) => s.settings.vsSupabaseKey);
  const flights = useSimBriefStore((s) => s.flights);
  const start = useLiveVsStore((s) => s.start);
  const stop = useLiveVsStore((s) => s.stop);
  const broadcastPos = useLiveVsStore((s) => s.broadcastPos);
  const broadcastLanding = useLiveVsStore((s) => s.broadcastLanding);
  const setSelfAircraft = useLiveVsStore((s) => s.setSelfAircraft);

  const [identity, setIdentity] = useState<{ identity: string; name: string } | null>(
    null,
  );

  // Identidad del piloto (daniel/hector) una vez.
  useEffect(() => {
    api
      .pilotProfile()
      .then((p) => setIdentity({ identity: p.identity, name: p.name }))
      .catch(() => {});
  }, []);

  // Credenciales: las de Ajustes mandan; si están vacías, usamos las
  // embebidas por defecto. La URL se normaliza (quita /rest/v1).
  const u = normalizeSupabaseUrl((url ?? "").trim() || DEFAULT_VS_SUPABASE_URL);
  const k = (key ?? "").trim() || DEFAULT_VS_SUPABASE_KEY;

  // (v6.2.13) Cuenta SimBrief COMPARTIDA: la lista trae los OFP de los dos
  // pilotos. Elegimos el que ME corresponde por la regla de callsign (Héctor =
  // "357", el resto = Daniel) en vez del más reciente — así nunca reclamo el
  // vuelo del otro ni se cruzan los números en la base de datos. Si todavía no
  // sé mi identidad, no arranco (para no subir un OFP equivocado).
  const ofp =
    identity != null
      ? (flights.find((f) => vsCallsignOwner(f.callsign) === identity.identity) ??
        ((identity.identity !== "hector" && identity.identity !== "daniel"
          ? flights[0]
          : undefined)))
      : undefined;

  // Arrancar / detener el canal cuando cambian credenciales, identidad u OFP.
  useEffect(() => {
    if (!u || !k || !identity || !ofp) {
      stop();
      return;
    }
    // (v6.2.13) Canal por DÍA de calendario (UTC) + ruta — NO por el timestamp
    // del OFP. Así los OFP distintos de cada piloto se encuentran en el mismo
    // duelo y el nombre coincide con el que usa el histórico del FlightBook
    // (que arma el canal con la fecha del vuelo). Antes usaba el epoch del OFP
    // → nunca cuadraba y el botón de Crew VS quedaba apagado.
    const date = new Date().toISOString().slice(0, 10);
    if (!ofp.originIcao || !ofp.destinationIcao || !date) {
      stop();
      return;
    }
    const vsOfp: VsOfp = {
      origin: ofp.originIcao,
      dest: ofp.destinationIcao,
      date,
      aircraft: ofp.aircraftIcao,
      registration: ofp.aircraftReg ?? null,
      callsign: ofp.callsign,
      flightNumber: ofp.flightNumber,
    };
    start(u, k, { identity: identity.identity, name: identity.name, ofp: vsOfp });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    u,
    k,
    identity?.identity,
    ofp?.ofpId,
    ofp?.originIcao,
    ofp?.destinationIcao,
  ]);

  // Emitir la posición en vivo cada 1.5s (el store no-opera si no hay canal).
  useEffect(() => {
    const interval = setInterval(() => {
      const s = useFlightLogStore.getState().status;
      // (v6.2.2) Fija el avión REAL en vuelo (matrícula/aerolínea del sim) antes
      // de difundir la posición — así la ficha muestra la foto correcta y el
      // rival recibe el avión real, no el del OFP.
      if (s?.aircraftRegistration || s?.aircraftAirline || s?.aircraftIcao) {
        setSelfAircraft({
          registration: s.aircraftRegistration ?? null,
          aircraftType: s.aircraftIcao ?? null,
          airlineName: s.aircraftAirline ?? null,
        });
      }
      if (s?.currentLat != null && s.currentLon != null) {
        broadcastPos(
          {
            lat: s.currentLat,
            lon: s.currentLon,
            heading: s.currentHeadingDeg ?? 0,
            alt: s.currentAltFt ?? 0,
            gs: s.currentGroundSpeedKt ?? 0,
          },
          s.phaseLabel ?? null,
        );
      }
    }, 1500);
    return () => clearInterval(interval);
  }, [broadcastPos, setSelfAircraft]);

  // (v6.1 #34) Al tocar pista el watcher emite `landing://osd` con el FPM/grado.
  // Lo retransmitimos al rival para la comparativa final del duelo.
  useEffect(() => {
    if (!isTauri) return;
    const un = listen<{ fpm: number; grade: string }>("landing://osd", (e) => {
      const p = e.payload;
      if (p && typeof p.fpm === "number") {
        broadcastLanding({ fpm: p.fpm, grade: p.grade });
      }
    });
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, [broadcastLanding]);

  return null;
}
