import { motion } from "framer-motion";
import { Plane, Radio } from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";

/**
 * Badge "Volando ahora" — visible en el header cuando MSFS
 * está corriendo. Tres niveles de información según lo que
 * tengamos disponible:
 *
 *   1. **SimConnect activo** (mejor caso): icono `Radio` + status
 *      granular ("Cruise", "Taxi out", "Deboarding"…). En vuelo
 *      muestra: ORIG → DEST · 234nm · ETA 12:34 · FL340 · 456kt.
 *      En tierra muestra: ORIG → DEST · Gate A34 · phase label.
 *   2. **Proceso MSFS + OFP**: icono `Plane` + ICAOs del plan.
 *   3. **Solo proceso**: "MSFS · sin OFP reciente".
 *
 * (v3.0.0) Refactor: muestra ETA + FL + GS + NM restantes cuando
 * el OFP de SimBrief coincide con el vuelo en curso. La distancia
 * y ETA se calculan client-side desde la posición actual hasta
 * destinationLat/Lon del OFP.
 */
export function FlyingNowBadge() {
  const status = useFlightLogStore((s) => s.status);
  const flights = useSimBriefStore((s) => s.flights);

  // (v3.4.13) Guard reforzado: además de simRunning, requerimos
  // que el watcher ya haya resuelto la posición real del avión
  // (currentAirportIcao). Antes el badge se renderizaba mientras
  // MSFS estaba en menú principal → tooltip y label sin sentido.
  if (!status || !status.simRunning) return null;
  const live = status.simconnectConnected;
  if (live && !status.currentAirportIcao && !status.originIcao) {
    return null;
  }
  // (v3.4.13) Match SimBrief OFP — ahora aceptamos match contra
  // `originIcao` del watcher (post-OUT) o `currentAirportIcao`
  // (preflight). Esto hace que el destino aparezca en el header
  // incluso antes de disparar el OUT.
  const matchedOfp = (() => {
    if (!flights.length) return null;
    const sorted = [...flights].sort((a, b) => {
      const aTs = a.generatedAt ? parseInt(a.generatedAt, 10) : 0;
      const bTs = b.generatedAt ? parseInt(b.generatedAt, 10) : 0;
      return bTs - aTs;
    });
    const watcherOrigin = status.originIcao || status.currentAirportIcao;
    if (!watcherOrigin) return null;
    return sorted.find((f) => f.originIcao === watcherOrigin) ?? null;
  })();
  // (v3.4.13) Origen y destino efectivos.
  //   · origen — prioridad: status.originIcao (watcher detectó OUT)
  //     > currentAirportIcao (avión spawneado, aún sin OUT).
  //   · destino — prioridad: status.destinationIcao (raro en preflight)
  //     > matchedOfp.destinationIcao (lo más común). Si no hay OFP que
  //     matchee y el avión no está en vuelo, el destino queda null y
  //     mostramos sólo "AT KATL".
  const effectiveOrigin =
    status.originIcao ?? status.currentAirportIcao ?? null;
  const effectiveDestination =
    status.destinationIcao ?? matchedOfp?.destinationIcao ?? null;
  const hasFullPlan = !!effectiveOrigin && !!effectiveDestination;
  const hasFlight = hasFullPlan;
  const phase = phaseDisplay(status.phaseLabel);

  // NM restantes desde la posición actual al destino del OFP.
  const nmRemaining = (() => {
    if (!live) return null;
    if (status.currentLat == null || status.currentLon == null) return null;
    if (!matchedOfp) return null;
    return haversineNm(
      status.currentLat,
      status.currentLon,
      matchedOfp.destinationLat,
      matchedOfp.destinationLon,
    );
  })();

  // ETA al destino en formato HH:MM (hora LOCAL del PC del usuario).
  // Usa GS actual y NM restantes. Si gs < 30kt o estamos en tierra,
  // no muestra ETA. Cambio v3.5.0 — antes era UTC con sufijo "Z".
  const etaLocal = (() => {
    if (nmRemaining == null) return null;
    if (status.currentGroundSpeedKt == null) return null;
    if (status.currentGroundSpeedKt < 30) return null;
    const hours = nmRemaining / status.currentGroundSpeedKt;
    const arrivalMs = Date.now() + hours * 3_600_000;
    const date = new Date(arrivalMs);
    const hh = date.getHours().toString().padStart(2, "0");
    const mm = date.getMinutes().toString().padStart(2, "0");
    return `${hh}:${mm}`;
  })();

  // FL (flight level): altitud en hundreds of feet, redondeada.
  // Sólo aplica > FL100; debajo mostramos altitud cruda.
  const flText = (() => {
    if (status.currentAltFt == null) return null;
    if (status.currentAltFt >= 10_000) {
      const fl = Math.round(status.currentAltFt / 100);
      return `FL${fl.toString().padStart(3, "0")}`;
    }
    return `${status.currentAltFt.toLocaleString("en-US")}ft`;
  })();

  const gsFmt =
    status.currentGroundSpeedKt != null && status.currentGroundSpeedKt > 0
      ? `${status.currentGroundSpeedKt}kt`
      : null;

  const onGround = !!status.onGround;
  const showGate = onGround && status.currentGate;

  // (v3.4.13) Tooltip más informativo + usa effective* en vez de
  // los campos crudos.
  const tooltip = live
    ? `SimConnect · ${phase.long}${
        effectiveOrigin && effectiveDestination
          ? ` · ${effectiveOrigin} → ${effectiveDestination}`
          : effectiveOrigin
            ? ` · AT ${effectiveOrigin}`
            : ""
      }${flText ? ` · ${flText}` : ""}${
        gsFmt ? ` · ${gsFmt}` : ""
      }${onGround ? " · on ground" : " · airborne"}`
    : hasFullPlan
      ? `MSFS detected (process) · SimBrief: ${effectiveOrigin} → ${effectiveDestination}`
      : effectiveOrigin
        ? `MSFS detected · AT ${effectiveOrigin}`
        : "MSFS detected but no recent OFP and SimConnect is not connected";

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.2 }}
      className={`inline-flex items-center gap-2 rounded-lg border px-3 py-1.5 text-xs font-medium ${phase.toneClass}`}
      title={tooltip}
    >
      <motion.span
        animate={{ opacity: [0.4, 1, 0.4] }}
        transition={{ duration: 1.6, repeat: Infinity }}
        className={`inline-block h-1.5 w-1.5 rounded-full ${phase.dotClass}`}
      />
      {live ? (
        <Radio className="h-3.5 w-3.5" />
      ) : (
        <Plane className="h-3.5 w-3.5" />
      )}
      {/* (v3.4.13) Mostramos efectivos: origen siempre que exista,
          destino si el OFP matchee O si el watcher lo conoce. */}
      {effectiveOrigin && effectiveDestination ? (
        <span className="font-mono">
          {effectiveOrigin}
          <span className={`mx-1.5 ${phase.arrowClass}`}>→</span>
          {effectiveDestination}
        </span>
      ) : effectiveOrigin ? (
        <span className="font-mono">AT {effectiveOrigin}</span>
      ) : (
        <span>{live ? phase.short : "MSFS · no OFP"}</span>
      )}

      {/* En tierra: gate + phase label. */}
      {live && hasFlight && showGate && (
        <span className={`font-mono text-[10px] ${phase.subClass}`}>
          · {status.currentGate}
        </span>
      )}
      {live && hasFlight && phase.short && (
        <span className={`text-[10px] uppercase tracking-wide ${phase.subClass}`}>
          · {phase.short}
        </span>
      )}

      {/* En vuelo: NM restantes + ETA. */}
      {live && nmRemaining != null && !onGround && (
        <span className={`font-mono text-[10px] ${phase.subClass}`}>
          · {Math.round(nmRemaining)}nm
        </span>
      )}
      {live && etaLocal && !onGround && (
        <span className={`font-mono text-[10px] ${phase.subClass}`}>
          · ETA {etaLocal}
        </span>
      )}

      {/* Siempre (en vivo): FL + GS. */}
      {live && flText && (
        <span className={`font-mono text-[10px] ${phase.subClass}`}>
          · {flText}
        </span>
      )}
      {live && gsFmt && (
        <span className={`font-mono text-[10px] ${phase.subClass}`}>
          · {gsFmt}
        </span>
      )}

      {/* Sin SimConnect: distancia total del plan. */}
      {!live && status.distanceNm != null && hasFlight && (
        <span className="text-[10px] text-emerald-300/70">
          · {status.distanceNm}nm
        </span>
      )}
    </motion.div>
  );
}

/** Distancia great-circle en millas náuticas. */
function haversineNm(
  lat1: number,
  lon1: number,
  lat2: number,
  lon2: number,
): number {
  const R = 3440.065; // radio Tierra en NM
  const toRad = (d: number) => (d * Math.PI) / 180;
  const dLat = toRad(lat2 - lat1);
  const dLon = toRad(lon2 - lon1);
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(a));
}

/**
 * Traduce el `phaseLabel` del backend a un display amigable + paleta
 * de colores adecuada para cada fase. Por defecto (sin phase o
 * fallback de proceso-only) usa la paleta verde original.
 */
function phaseDisplay(phase: string | null): {
  short: string;
  long: string;
  toneClass: string;
  dotClass: string;
  arrowClass: string;
  subClass: string;
} {
  // Paletas: sky (en tierra preparando) / emerald (en vuelo activo) /
  // amber (taxi, transiciones) / slate (preflight/deboarding).
  const palettes = {
    slate: {
      toneClass: "border-slate-600/60 bg-slate-700/30 text-slate-200",
      dotClass: "bg-slate-300",
      arrowClass: "text-slate-400",
      subClass: "text-slate-400",
    },
    sky: {
      toneClass: "border-sky-400/60 bg-sky-500/15 text-sky-100",
      dotClass: "bg-sky-300",
      arrowClass: "text-sky-400",
      subClass: "text-sky-300/80",
    },
    amber: {
      toneClass: "border-amber-400/60 bg-amber-500/15 text-amber-100",
      dotClass: "bg-amber-300",
      arrowClass: "text-amber-400",
      subClass: "text-amber-300/80",
    },
    emerald: {
      toneClass: "border-emerald-400/60 bg-emerald-500/15 text-emerald-100",
      dotClass: "bg-emerald-300",
      arrowClass: "text-emerald-400",
      subClass: "text-emerald-300/80",
    },
  };

  // (v2.2.0) Labels en INGLÉS — pedido explícito del usuario "Todos
  // absolutamente todos los labels deben de ser en ingles".
  switch (phase) {
    case "preflight":
      return { short: "Pre-flight", long: "Pre-flight (cold and dark)", ...palettes.slate };
    case "engine_running":
      return { short: "Engines on", long: "Engines running at gate", ...palettes.sky };
    case "pushback":
      return { short: "Pushback", long: "Pushback in progress", ...palettes.amber };
    case "taxi_out":
      return { short: "Taxi out", long: "Taxi to runway", ...palettes.amber };
    case "takeoff":
      return { short: "Takeoff", long: "Takeoff roll", ...palettes.emerald };
    case "climbing":
      return { short: "Climbing", long: "Climbing", ...palettes.emerald };
    case "cruise":
      return { short: "Cruise", long: "Cruise", ...palettes.emerald };
    case "descent":
      return { short: "Descent", long: "Descending", ...palettes.emerald };
    case "approach":
      return { short: "Approach", long: "Final approach", ...palettes.emerald };
    case "landed_rollout":
      return { short: "Landing", long: "Landing roll-out", ...palettes.amber };
    case "taxi_in":
      return { short: "Taxi in", long: "Taxi to gate", ...palettes.amber };
    case "parking":
      return { short: "At gate", long: "At gate", ...palettes.sky };
    case "deboarding":
      return { short: "Deboarding", long: "Deboarding (engines off)", ...palettes.sky };
    default:
      return { short: "Flying", long: "Flight active", ...palettes.emerald };
  }
}
