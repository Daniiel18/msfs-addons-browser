import { Cog } from "lucide-react";
import { t } from "../lib/i18n";

/**
 * (v5.0.0) Micro-badge "optimizable": una tuerca en color distintivo
 * (violeta) que marca los escenarios a los que SÍ se les puede aplicar
 * el optimizador de FPS (tienen objetos opcionales desactivables). Si
 * un aeropuerto no lo lleva, es que no tiene objetos opcionales (o no se
 * detectaron). Compartido por la sidebar del Map, la card y el Link Map.
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
      className={`inline-flex items-center gap-0.5 rounded bg-violet-500/20 px-1 py-0.5 text-[9px] font-bold uppercase tracking-wide text-violet-200 ring-1 ring-violet-400/40 ${className}`}
    >
      <Cog className="h-2.5 w-2.5" />
      {showLabel && "FPS"}
    </span>
  );
}
