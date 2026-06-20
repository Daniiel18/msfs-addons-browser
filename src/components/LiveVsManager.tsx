import { useEffect, useState } from "react";
import { api } from "../lib/tauri";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useLiveVsStore, type VsOfp } from "../stores/useLiveVsStore";
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
  const ofp = flights[0]; // OFP más reciente (ya filtrado por propiedad/correo)

  // Arrancar / detener el canal cuando cambian credenciales, identidad u OFP.
  useEffect(() => {
    if (!u || !k || !identity || !ofp) {
      stop();
      return;
    }
    const date = (ofp.generatedAt ?? ofp.fetchedAt ?? "").slice(0, 10);
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
      if (s?.currentLat != null && s.currentLon != null) {
        broadcastPos({
          lat: s.currentLat,
          lon: s.currentLon,
          heading: s.currentHeadingDeg ?? 0,
          alt: s.currentAltFt ?? 0,
          gs: s.currentGroundSpeedKt ?? 0,
        });
      }
    }, 1500);
    return () => clearInterval(interval);
  }, [broadcastPos]);

  return null;
}
