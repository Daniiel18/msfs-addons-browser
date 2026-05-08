import { useState } from "react";
import { Loader2, Plane, RefreshCw, Settings, Trash2, X } from "lucide-react";
import { useSimBriefStore } from "../stores/useSimBriefStore";

/**
 * Tarjeta colapsable embebida en la sidebar del mapa con:
 *   · Configuración del SimBrief Pilot ID (input + guardar).
 *   · Botón refrescar — descarga el último OFP y lo persiste.
 *   · Lista de los últimos vuelos guardados con `Origin → Destination`,
 *     callsign / flight number y botón "borrar" por si el usuario
 *     metió un OFP de prueba.
 *
 * Las líneas en el mapa se dibujan desde `MapView` que también
 * lee `useSimBriefStore.flights`.
 */
export function SimBriefPanel() {
  const pilotId = useSimBriefStore((s) => s.pilotId);
  const flights = useSimBriefStore((s) => s.flights);
  const refreshing = useSimBriefStore((s) => s.refreshing);
  const lastError = useSimBriefStore((s) => s.lastError);
  const setPilotId = useSimBriefStore((s) => s.setPilotId);
  const refresh = useSimBriefStore((s) => s.refresh);
  const remove = useSimBriefStore((s) => s.remove);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const startEdit = () => {
    setDraft(pilotId ?? "");
    setEditing(true);
  };
  const save = async () => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    await setPilotId(trimmed);
    setEditing(false);
  };

  return (
    <section className="border-b border-slate-800 bg-slate-900/30 px-3 py-2.5">
      <header className="mb-2 flex items-center justify-between gap-2">
        <div className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-slate-300">
          <Plane className="h-3 w-3 text-sky-300" />
          SimBrief
          {flights.length > 0 && (
            <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-normal text-slate-400">
              {flights.length}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={startEdit}
            title="Configurar Pilot ID"
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <Settings className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={refresh}
            disabled={refreshing || !pilotId}
            title={
              pilotId
                ? "Descargar el último OFP de SimBrief"
                : "Configura tu Pilot ID primero"
            }
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-40"
          >
            {refreshing ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </div>
      </header>

      {editing && (
        <div className="mb-2 rounded-md border border-slate-800 bg-slate-950/60 p-2">
          <label className="block text-[10px] uppercase tracking-wide text-slate-500">
            Pilot ID
          </label>
          <p className="mt-0.5 text-[10px] text-slate-500">
            En SimBrief: <span className="font-mono">Account &rarr; Pilot ID</span>.
          </p>
          <div className="mt-1 flex items-center gap-1">
            <input
              type="text"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="50956"
              className="w-full rounded-md border border-slate-800 bg-slate-950/80 px-2 py-1 text-xs text-slate-100 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30"
            />
            <button
              onClick={save}
              className="rounded-md bg-brand-500/80 px-2 py-1 text-xs font-medium text-slate-100 hover:bg-brand-500"
            >
              Guardar
            </button>
            <button
              onClick={() => setEditing(false)}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-800"
              title="Cancelar"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      )}

      {!pilotId && !editing && (
        <p className="mb-2 text-[11px] text-slate-500">
          Configura tu SimBrief Pilot ID para ver tus vuelos en el mapa.
        </p>
      )}

      {lastError && (
        <p className="mb-2 rounded bg-rose-500/15 px-2 py-1 text-[10px] text-rose-200 ring-1 ring-rose-500/30">
          {lastError}
        </p>
      )}

      {flights.length > 0 && (
        <ul className="max-h-40 overflow-y-auto space-y-0.5">
          {flights.slice(0, 12).map((f) => (
            <li
              key={f.ofpId}
              className="group flex items-center justify-between gap-1 rounded px-1.5 py-1 text-[11px] text-slate-300 hover:bg-slate-800/50"
            >
              <span className="font-mono text-slate-200">
                {f.originIcao} <span className="text-sky-300">→</span> {f.destinationIcao}
              </span>
              <span className="ml-auto truncate text-[10px] text-slate-500">
                {f.flightNumber ?? f.callsign ?? f.aircraftIcao ?? ""}
              </span>
              <button
                onClick={() => remove(f.ofpId)}
                title="Eliminar este vuelo"
                className="rounded p-0.5 text-slate-600 opacity-0 transition-opacity hover:text-rose-300 group-hover:opacity-100"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            </li>
          ))}
          {flights.length > 12 && (
            <li className="px-1.5 text-[10px] italic text-slate-500">
              … y {flights.length - 12} más
            </li>
          )}
        </ul>
      )}
    </section>
  );
}
