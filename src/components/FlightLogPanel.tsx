import { Activity, AlertCircle, Loader2, Plane, Trash2 } from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";

/**
 * Panel embebido en la sidebar del mapa con el historial de vuelos
 * reales (SimConnect). Hermano de `SimBriefPanel` — ambos comparten
 * estilo y se apilan en la misma sidebar.
 *
 * El watcher real aún no está cableado (stub), así que el listado
 * estará vacío hasta que se inserten entradas con
 * `__seedFlightLog()` desde la consola o se active SimConnect.
 */
export function FlightLogPanel() {
  const entries = useFlightLogStore((s) => s.entries);
  const loading = useFlightLogStore((s) => s.loading);
  const lastError = useFlightLogStore((s) => s.lastError);
  const reload = useFlightLogStore((s) => s.reload);
  const remove = useFlightLogStore((s) => s.remove);

  const inFlight = entries.find((e) => e.endedAt === null);

  return (
    <section className="border-b border-slate-800 bg-slate-900/30 px-3 py-2.5">
      <header className="mb-2 flex items-center justify-between gap-2">
        <div className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-slate-300">
          <Activity className="h-3 w-3 text-emerald-300" />
          SimConnect
          {entries.length > 0 && (
            <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-normal text-slate-400">
              {entries.length}
            </span>
          )}
        </div>
        <button
          onClick={reload}
          title="Recargar historial"
          className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Plane className="h-3.5 w-3.5" />
          )}
        </button>
      </header>

      {lastError && (
        <div className="mb-2 flex items-start gap-1.5 rounded bg-rose-500/15 px-2 py-1 text-[10px] text-rose-200 ring-1 ring-rose-500/30">
          <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
          <span>{lastError}</span>
        </div>
      )}

      {inFlight && (
        <div className="mb-2 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-200">
          <div className="font-medium">En vuelo ahora</div>
          <div className="text-[10px] text-emerald-300/80">
            {inFlight.originIcao ?? "?"} ·{" "}
            {inFlight.aircraftAtcType ??
              inFlight.aircraftTitle ??
              "Aircraft desconocido"}
          </div>
        </div>
      )}

      {entries.length === 0 ? (
        <p className="text-[11px] text-slate-500">
          Sin vuelos registrados todavía. Cuando despegues en MSFS, el
          watcher creará una entrada automáticamente (próxima
          versión).
        </p>
      ) : (
        <ul className="max-h-40 space-y-0.5 overflow-y-auto">
          {entries.slice(0, 12).map((e) => (
            <li
              key={e.id}
              className="group flex items-center justify-between gap-1 rounded px-1.5 py-1 text-[11px] text-slate-300 hover:bg-slate-800/50"
            >
              <span className="font-mono text-slate-200">
                {e.originIcao ?? "?"}{" "}
                <span className="text-emerald-300">→</span>{" "}
                {e.destinationIcao ?? (e.endedAt ? "?" : "…")}
              </span>
              <span className="ml-auto truncate text-[10px] text-slate-500">
                {e.distanceNm != null && `${Math.round(e.distanceNm)}nm`}
                {e.aircraftAtcType && ` · ${e.aircraftAtcType}`}
              </span>
              <button
                onClick={() => remove(e.id)}
                title="Eliminar este vuelo"
                className="rounded p-0.5 text-slate-600 opacity-0 transition-opacity hover:text-rose-300 group-hover:opacity-100"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            </li>
          ))}
          {entries.length > 12 && (
            <li className="px-1.5 text-[10px] italic text-slate-500">
              … y {entries.length - 12} más
            </li>
          )}
        </ul>
      )}
    </section>
  );
}
