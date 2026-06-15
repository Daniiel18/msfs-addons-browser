import { Wrench } from "lucide-react";
import { t } from "../lib/i18n";

/**
 * (v5.0.0) Micro-badge "optimizable": una llave inglesa AMARILLA que
 * marca los escenarios a los que SÍ se les puede aplicar el optimizador
 * de FPS (tienen "Optional Configuration" en SceneryAddons). Si un
 * aeropuerto no lo lleva, no tiene opciones y su modal queda bloqueado.
 * Compartido por el Link Map (nodos de aeropuerto).
 */
export function PerfBadge({
  className = "",
  showLabel = true,
}: {
  className?: string;
  showLabel?: boolean;
}) {
  return (
    <span
      title={t("perf.badge_title")}
      className={`inline-flex items-center gap-0.5 rounded-md bg-amber-400/90 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-amber-950 shadow-sm ring-1 ring-amber-300 ${className}`}
    >
      <Wrench className="h-3 w-3" />
      {showLabel && "FPS"}
    </span>
  );
}
