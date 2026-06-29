import { useEffect, useState } from "react";
import { Activity, Lock, RefreshCw } from "lucide-react";
import { api, type DiagProcReport } from "../lib/tauri";

/**
 * (v6.2.15) Panel de diagnóstico de memoria — detrás de una clave de acceso.
 * Lista TODOS los procesos del árbol de SimFleet (binario principal + procesos
 * hijo del WebView2 + lo que la app lance) con la RAM de cada uno, para cazar
 * fugas. Cada consulta vuelca además el reporte al log (`target: "diag"`).
 */

const ACCESS_CODE = "3875";

function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

export function DiagnosticsSettings() {
  const [unlocked, setUnlocked] = useState(false);
  const [code, setCode] = useState("");
  const [error, setError] = useState(false);
  const [report, setReport] = useState<DiagProcReport | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = () => {
    setLoading(true);
    api
      .diagnosticsProcessTree()
      .then((r) => setReport(r))
      .catch((e) => console.warn("diagnosticsProcessTree falló:", e))
      .finally(() => setLoading(false));
  };

  // Auto-refresca cada 2s mientras esté desbloqueado.
  useEffect(() => {
    if (!unlocked) return;
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [unlocked]);

  if (!unlocked) {
    return (
      <div className="space-y-2 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-3">
        <div className="flex items-center gap-1.5 text-[11px] text-slate-400">
          <Lock className="h-3.5 w-3.5" />
          Introduce la clave de acceso para ver el diagnóstico de procesos y RAM.
        </div>
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (code.trim() === ACCESS_CODE) {
              setUnlocked(true);
              setError(false);
            } else {
              setError(true);
            }
          }}
        >
          <input
            type="password"
            inputMode="numeric"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="Clave"
            className="w-32 rounded-md border border-slate-700 bg-slate-950 px-2 py-1 text-xs text-slate-200 outline-none focus:border-brand-500"
          />
          <button
            type="submit"
            className="rounded-md bg-brand-500 px-3 py-1 text-xs font-medium text-white hover:bg-brand-400"
          >
            Desbloquear
          </button>
        </form>
        {error && (
          <p className="text-[11px] text-rose-400">Clave incorrecta.</p>
        )}
      </div>
    );
  }

  const procs = report?.processes ?? [];

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2">
        <div className="flex items-center gap-1.5 text-[11px] text-slate-400">
          <Activity className="h-3.5 w-3.5 text-emerald-400" />
          <span>
            Total SimFleet:{" "}
            <span className="font-mono font-semibold text-slate-100">
              {report ? fmtMB(report.totalBytes) : "—"}
            </span>{" "}
            <span className="text-slate-500">({report?.count ?? 0} procesos)</span>
          </span>
        </div>
        <button
          onClick={refresh}
          className="inline-flex items-center gap-1 rounded-md border border-slate-700 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-800"
        >
          <RefreshCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
          Refrescar
        </button>
      </div>

      <div className="overflow-hidden rounded-md border border-slate-800">
        <table className="w-full text-[11px]">
          <thead>
            <tr className="bg-slate-900/60 text-slate-500">
              <th className="px-2 py-1.5 text-left font-medium">Proceso</th>
              <th className="px-2 py-1.5 text-right font-medium">PID</th>
              <th className="px-2 py-1.5 text-right font-medium">RAM</th>
            </tr>
          </thead>
          <tbody>
            {procs.map((p) => (
              <tr
                key={p.pid}
                className="border-t border-slate-800/70 odd:bg-slate-950/40"
              >
                <td className="truncate px-2 py-1.5 text-slate-200">
                  {p.isMain && (
                    <span className="mr-1 rounded bg-brand-500/20 px-1 text-[9px] font-bold uppercase text-brand-300">
                      main
                    </span>
                  )}
                  {p.name}
                </td>
                <td className="px-2 py-1.5 text-right font-mono text-slate-500">
                  {p.pid}
                </td>
                <td className="px-2 py-1.5 text-right font-mono text-slate-100">
                  {fmtMB(p.memoryBytes)}
                </td>
              </tr>
            ))}
            {procs.length === 0 && (
              <tr>
                <td
                  colSpan={3}
                  className="px-2 py-3 text-center text-[11px] text-slate-500"
                >
                  Sin datos todavía…
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="text-[10px] text-slate-500">
        Cada lectura se guarda también en el log de la app (etiqueta{" "}
        <span className="font-mono">diag</span>). Comparte el log si quieres que
        revise el proceso que más consume.
      </p>
    </div>
  );
}
