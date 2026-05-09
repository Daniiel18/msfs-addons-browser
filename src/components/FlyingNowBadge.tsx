import { motion } from "framer-motion";
import { Plane } from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";

/**
 * Badge "Volando ahora" — visible en el header sólo cuando:
 *   1. El watcher detecta el proceso de MSFS corriendo, Y
 *   2. Hay una OFP fresca (< 6 h) en el historial de SimBrief.
 *
 * Cuando MSFS está abierto sin OFP fresca, el badge muestra
 * "MSFS corriendo · sin OFP" para confirmar al menos la detección
 * del proceso.
 *
 * Si MSFS no está abierto, el componente devuelve `null` (no se
 * renderiza nada).
 */
export function FlyingNowBadge() {
  const status = useFlightLogStore((s) => s.status);
  if (!status || !status.simRunning) return null;

  const hasFlight = !!status.originIcao && !!status.destinationIcao;

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.2 }}
      className="inline-flex items-center gap-2 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-3 py-1.5 text-xs font-medium text-emerald-200"
      title={
        hasFlight
          ? `Vuelo activo (de la última OFP de SimBrief): ${status.originIcao} → ${status.destinationIcao}${status.aircraftIcao ? ` · ${status.aircraftIcao}` : ""}`
          : "MSFS detectado pero no hay OFP reciente en SimBrief"
      }
    >
      <motion.span
        animate={{ opacity: [0.4, 1, 0.4] }}
        transition={{ duration: 1.6, repeat: Infinity }}
        className="inline-block h-1.5 w-1.5 rounded-full bg-emerald-400"
      />
      <Plane className="h-3.5 w-3.5" />
      {hasFlight ? (
        <span className="font-mono">
          {status.originIcao}
          <span className="mx-1.5 text-emerald-400">→</span>
          {status.destinationIcao}
        </span>
      ) : (
        <span>MSFS · sin OFP reciente</span>
      )}
      {status.distanceNm != null && hasFlight && (
        <span className="text-[10px] text-emerald-300/70">
          · {status.distanceNm}nm
        </span>
      )}
    </motion.div>
  );
}
