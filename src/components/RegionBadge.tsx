import { airportRegion, regionBadgeClass } from "../lib/oaciRegion";

/**
 * (v5.0.0) Micro-badge de región/país a partir del ICAO. Color por
 * continente. Compartido por la card del Map, la sidebar y los nodos de
 * aeropuerto del Link Map para que el usuario identifique de un vistazo
 * a qué región pertenece cada aeropuerto.
 */
export function RegionBadge({
  icao,
  className = "",
  showLabel = true,
}: {
  icao: string | null | undefined;
  className?: string;
  /** Muestra el nombre del país junto al código (false = solo código). */
  showLabel?: boolean;
}) {
  const r = airportRegion(icao);
  return (
    <span
      title={r.label}
      className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide shadow-sm ring-1 ${regionBadgeClass(
        r.continent,
      )} ${className}`}
    >
      <span className="font-mono">{r.code}</span>
      {showLabel && (
        <span className="max-w-[10rem] truncate font-semibold normal-case tracking-normal">
          {r.label}
        </span>
      )}
    </span>
  );
}
