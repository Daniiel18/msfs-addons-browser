import { motion } from "framer-motion";
import { Plane, Radio } from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";

/**
 * Badge "Volando ahora" — visible en el header cuando MSFS
 * está corriendo. Tres niveles de información según lo que
 * tengamos disponible:
 *
 *   1. **SimConnect activo** (mejor caso): icono `Radio` +
 *      coords en vivo (alt + groundspeed). Si además hay OFP
 *      fresco, también mostramos origen→destino.
 *   2. **Proceso MSFS + OFP**: icono `Plane` + ICAOs del plan.
 *   3. **Solo proceso**: "MSFS · sin OFP reciente".
 */
export function FlyingNowBadge() {
  const status = useFlightLogStore((s) => s.status);
  if (!status || !status.simRunning) return null;

  const hasFlight = !!status.originIcao && !!status.destinationIcao;
  const live = status.simconnectConnected;
  const altFmt =
    status.currentAltFt != null
      ? `${status.currentAltFt.toLocaleString("es-ES")}ft`
      : null;
  const gsFmt =
    status.currentGroundSpeedKt != null && status.currentGroundSpeedKt > 0
      ? `${status.currentGroundSpeedKt}kt`
      : null;

  const tooltip = live
    ? `SimConnect en vivo · alt ${altFmt ?? "?"} · gs ${gsFmt ?? "0"}${
        status.onGround ? " · en tierra" : " · en vuelo"
      }`
    : hasFlight
      ? `MSFS detectado (proceso) · plan SimBrief: ${status.originIcao} → ${status.destinationIcao}`
      : "MSFS detectado pero no hay OFP reciente y SimConnect no está conectado";

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.2 }}
      className={`inline-flex items-center gap-2 rounded-lg border px-3 py-1.5 text-xs font-medium ${
        live
          ? "border-emerald-400/60 bg-emerald-500/15 text-emerald-100"
          : "border-emerald-500/40 bg-emerald-500/10 text-emerald-200"
      }`}
      title={tooltip}
    >
      <motion.span
        animate={{ opacity: [0.4, 1, 0.4] }}
        transition={{ duration: 1.6, repeat: Infinity }}
        className={`inline-block h-1.5 w-1.5 rounded-full ${
          live ? "bg-emerald-300" : "bg-emerald-400"
        }`}
      />
      {live ? (
        <Radio className="h-3.5 w-3.5" />
      ) : (
        <Plane className="h-3.5 w-3.5" />
      )}
      {hasFlight ? (
        <span className="font-mono">
          {status.originIcao}
          <span className="mx-1.5 text-emerald-400">→</span>
          {status.destinationIcao}
        </span>
      ) : (
        <span>{live ? "Volando" : "MSFS · sin OFP"}</span>
      )}
      {live && altFmt && (
        <span className="font-mono text-[10px] text-emerald-300/80">
          · {altFmt}
        </span>
      )}
      {live && gsFmt && (
        <span className="font-mono text-[10px] text-emerald-300/80">
          · {gsFmt}
        </span>
      )}
      {!live && status.distanceNm != null && hasFlight && (
        <span className="text-[10px] text-emerald-300/70">
          · {status.distanceNm}nm
        </span>
      )}
    </motion.div>
  );
}
