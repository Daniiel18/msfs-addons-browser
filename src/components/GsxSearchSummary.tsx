import { useEffect, useState } from "react";
import { ExternalLink, Loader2, Sparkles } from "lucide-react";
import { api } from "../lib/tauri";

interface Props {
  query: string;
}

/**
 * Una sola píldora debajo del buscador con el conteo de perfiles
 * GSX que existen en flightsim.to para el ICAO buscado. Click →
 * abre la búsqueda en flightsim.to con `?q=ICAO`.
 *
 * Sólo se renderiza cuando el query es exactamente un ICAO de 4
 * letras ASCII — para queries free-text (`"frankfurt"`, `"asobo"`)
 * no tiene sentido consultar GSX, esconder el badge evita
 * confusión.
 *
 * El backend (`gsx_lookup`) cachea 24h en SQLite, así que repetir
 * el mismo ICAO durante el día es instantáneo.
 */
export function GsxSearchSummary({ query }: Props) {
  const trimmed = query.trim();
  const isIcao = /^[A-Za-z]{4}$/.test(trimmed);
  const icao = trimmed.toUpperCase();

  const [count, setCount] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!isIcao) {
      setCount(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    api
      .gsxLookup(icao)
      .then((profiles) => {
        if (!cancelled) setCount(profiles.length);
      })
      .catch((e) => {
        if (cancelled) return;
        console.warn("gsxLookup failed for", icao, e);
        setCount(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isIcao, icao]);

  if (!isIcao) return null;

  const url = `https://flightsim.to/miscellaneous/gsx-pro?q=${encodeURIComponent(icao)}`;
  const handleClick = () => api.openExternal(url);

  if (loading || count === null) {
    return (
      <button
        onClick={handleClick}
        className="inline-flex items-center gap-2 rounded-lg bg-emerald-500/15 px-3 py-1.5 text-xs font-medium text-emerald-200 ring-1 ring-emerald-500/30 transition-colors hover:bg-emerald-500/25"
        title={`Abrir GSX para ${icao} en flightsim.to`}
      >
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        GSX · {icao}
      </button>
    );
  }

  return (
    <button
      onClick={handleClick}
      title={`${count} perfil${count === 1 ? "" : "es"} GSX para ${icao}. Click para abrir en flightsim.to`}
      className="group/gsx inline-flex items-center gap-2 rounded-lg bg-emerald-500 px-3 py-1.5 text-xs font-bold uppercase tracking-wide text-emerald-950 shadow-lg shadow-emerald-500/30 ring-1 ring-emerald-300 transition-all hover:scale-[1.03] hover:bg-emerald-400 hover:shadow-emerald-400/40"
    >
      <Sparkles className="h-3.5 w-3.5" />
      <span>
        GSX · {icao}
      </span>
      <span className="rounded bg-emerald-950/30 px-1.5 py-0.5 text-[10px] font-extrabold tabular-nums text-emerald-100">
        {count}
      </span>
      <ExternalLink className="h-3 w-3 opacity-70 transition-opacity group-hover/gsx:opacity-100" />
    </button>
  );
}
