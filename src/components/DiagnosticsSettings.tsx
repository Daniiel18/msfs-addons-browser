import { useEffect, useState } from "react";
import {
  Activity,
  Camera,
  Cloud,
  Delete,
  Lock,
  MapPin,
  Plug,
  RefreshCw,
  Swords,
} from "lucide-react";
import {
  api,
  type DiagProcReport,
  type DiagAppMetrics,
} from "../lib/tauri";
import { useLiveVsStore } from "../stores/useLiveVsStore";
import { t } from "../lib/i18n";

/**
 * (v6.2.15 · v6.2.16) Panel de diagnóstico — detrás de una clave de acceso.
 *  · Subsistemas INTERNOS de la app (SimConnect, tracking, nube, grabador,
 *    Crew VS) con su actividad en vivo — para cazar cuál trabaja de más.
 *  · Árbol de procesos del SO (simfleet.exe + WebView2) con la RAM de cada uno.
 * La clave se introduce con teclado PIN (clic) o escribiendo, y el desbloqueo
 * PERSISTE (localStorage) — no se vuelve a pedir al cambiar de sección ni al
 * cerrar el modal.
 */

const ACCESS_CODE = "3875";
/** Desbloqueo persistente — sobrevive a cerrar el modal / cambiar de sección. */
const UNLOCK_KEY = "simfleet.diag.unlocked.v1";

function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

function fmtAgo(nowMs: number, thenMs: number): string {
  if (!thenMs) return "—";
  const s = Math.max(0, Math.round((nowMs - thenMs) / 1000));
  if (s < 60) return t("diag.ago_s", { n: String(s) });
  return t("diag.ago_min", { n: String(Math.round(s / 60)) });
}

export function DiagnosticsSettings() {
  const [unlocked, setUnlocked] = useState<boolean>(() => {
    try {
      return localStorage.getItem(UNLOCK_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [code, setCode] = useState("");
  const [error, setError] = useState(false);
  const [report, setReport] = useState<DiagProcReport | null>(null);
  const [metrics, setMetrics] = useState<DiagAppMetrics | null>(null);
  const [loading, setLoading] = useState(false);

  // Estado del Crew VS (Supabase) — vive en el frontend, se lee del store.
  const vsConnected = useLiveVsStore((s) => s.connected);
  const vsChannel = useLiveVsStore((s) => s.currentChannel);
  const vsRival = useLiveVsStore((s) => s.rival);

  const tryCode = (value: string) => {
    if (value.trim() === ACCESS_CODE) {
      setUnlocked(true);
      setError(false);
      try {
        localStorage.setItem(UNLOCK_KEY, "1");
      } catch {
        /* ignore */
      }
    } else if (value.length >= ACCESS_CODE.length) {
      setError(true);
      setCode("");
    }
  };

  const pressDigit = (d: string) => {
    const next = (code + d).slice(0, 8);
    setCode(next);
    setError(false);
    tryCode(next);
  };

  const refresh = () => {
    setLoading(true);
    Promise.all([api.diagnosticsProcessTree(), api.diagnosticsAppMetrics()])
      .then(([r, m]) => {
        setReport(r);
        setMetrics(m);
      })
      .catch((e) => console.warn("diagnostics falló:", e))
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
      <div className="space-y-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-3">
        <div className="flex items-center gap-1.5 text-[11px] text-slate-400">
          <Lock className="h-3.5 w-3.5" />
          {t("diag.unlock.hint")}
        </div>

        {/* Display del PIN + entrada por teclado */}
        <input
          type="password"
          inputMode="numeric"
          autoComplete="off"
          value={code}
          onChange={(e) => {
            const v = e.target.value.replace(/\D/g, "").slice(0, 8);
            setCode(v);
            setError(false);
            tryCode(v);
          }}
          placeholder="····"
          className={`w-full rounded-md border bg-slate-950 px-3 py-2 text-center font-mono text-lg tracking-[0.5em] text-slate-100 outline-none ${
            error ? "border-rose-500/60" : "border-slate-700 focus:border-brand-500"
          }`}
        />

        {/* Teclado PIN clicable */}
        <div className="mx-auto grid w-44 grid-cols-3 gap-1.5">
          {["1", "2", "3", "4", "5", "6", "7", "8", "9"].map((d) => (
            <button
              key={d}
              type="button"
              onClick={() => pressDigit(d)}
              className="rounded-lg border border-slate-700 bg-slate-900 py-2 text-sm font-semibold text-slate-200 transition-colors hover:border-brand-500/50 hover:bg-slate-800 active:scale-95"
            >
              {d}
            </button>
          ))}
          <button
            type="button"
            onClick={() => {
              setCode("");
              setError(false);
            }}
            className="rounded-lg border border-slate-800 bg-slate-950 py-2 text-[10px] font-medium text-slate-500 hover:text-slate-300"
          >
            {t("diag.unlock.clear")}
          </button>
          <button
            type="button"
            onClick={() => pressDigit("0")}
            className="rounded-lg border border-slate-700 bg-slate-900 py-2 text-sm font-semibold text-slate-200 transition-colors hover:border-brand-500/50 hover:bg-slate-800 active:scale-95"
          >
            0
          </button>
          <button
            type="button"
            onClick={() => {
              setCode((c) => c.slice(0, -1));
              setError(false);
            }}
            className="flex items-center justify-center rounded-lg border border-slate-800 bg-slate-950 py-2 text-slate-500 hover:text-slate-300"
          >
            <Delete className="h-4 w-4" />
          </button>
        </div>

        {error && (
          <p className="text-center text-[11px] text-rose-400">
            {t("diag.unlock.wrong")}
          </p>
        )}
      </div>
    );
  }

  const procs = report?.processes ?? [];
  const m = metrics;

  return (
    <div className="space-y-2">
      {/* ── Subsistemas internos ── */}
      <p className="text-[10px] font-semibold uppercase tracking-wider text-slate-500">
        {t("diag.subsystems")}
      </p>
      <div className="grid grid-cols-2 gap-1.5">
        <SubsystemCard
          icon={<Plug className="h-3.5 w-3.5" />}
          title="SimConnect"
          ok={m?.simconnectConnected ?? false}
          status={m?.simconnectConnected ? t("diag.status.connected") : t("diag.status.disconnected")}
          detail={
            m
              ? t("diag.detail.emits", { n: m.simconnectEmits.toLocaleString(), ago: fmtAgo(m.nowMs, m.simconnectLastEmitMs) })
              : "—"
          }
        />
        <SubsystemCard
          icon={<MapPin className="h-3.5 w-3.5" />}
          title="Tracking"
          ok={(m?.trackRowsWritten ?? 0) > 0}
          status={m && m.trackRowsWritten > 0 ? t("diag.status.recording") : t("diag.status.idle")}
          detail={
            m
              ? t("diag.detail.points", { n: m.trackRowsWritten.toLocaleString() })
              : "—"
          }
        />
        <SubsystemCard
          icon={<Cloud className="h-3.5 w-3.5" />}
          title={t("diag.card.cloud")}
          ok={m?.cloudBusy ?? false}
          status={m?.cloudBusy ? t("diag.status.uploading") : t("diag.status.resting")}
          detail={
            m && m.cloudUploads > 0
              ? t("diag.detail.uploads", { n: String(m.cloudUploads), mb: fmtMB(m.cloudLastBytes), ago: fmtAgo(m.nowMs, m.cloudLastMs) })
              : t("diag.detail.no_uploads")
          }
        />
        <SubsystemCard
          icon={<Camera className="h-3.5 w-3.5" />}
          title={t("diag.card.recorder")}
          ok={m?.recorderArmed ?? false}
          status={m?.recorderArmed ? t("diag.status.armed") : t("diag.status.disarmed")}
          detail={
            m?.recorderArmed
              ? t("diag.detail.rec_on")
              : t("diag.detail.rec_off")
          }
          warnWhenOk
        />
        <SubsystemCard
          icon={<Swords className="h-3.5 w-3.5" />}
          title="Crew VS (Supabase)"
          ok={vsConnected}
          status={vsConnected ? t("diag.status.connected") : t("diag.status.no_channel")}
          detail={
            vsConnected
              ? `${vsChannel || "—"}${vsRival ? ` · ${t("diag.detail.rival")}: ${vsRival.name}` : ` · ${t("diag.detail.no_rival")}`}`
              : t("diag.detail.vs_off")
          }
        />
      </div>

      {/* ── Procesos del SO ── */}
      <div className="flex items-center justify-between rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2">
        <div className="flex items-center gap-1.5 text-[11px] text-slate-400">
          <Activity className="h-3.5 w-3.5 text-emerald-400" />
          <span>
            {t("diag.total")}{" "}
            <span className="font-mono font-semibold text-slate-100">
              {report ? fmtMB(report.totalBytes) : "—"}
            </span>{" "}
            <span className="text-slate-500">({report?.count ?? 0} {t("diag.processes")})</span>
          </span>
        </div>
        <button
          onClick={refresh}
          className="inline-flex items-center gap-1 rounded-md border border-slate-700 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-800"
        >
          <RefreshCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
          {t("diag.refresh")}
        </button>
      </div>

      <div className="overflow-hidden rounded-md border border-slate-800">
        <table className="w-full text-[11px]">
          <thead>
            <tr className="bg-slate-900/60 text-slate-500">
              <th className="px-2 py-1.5 text-left font-medium">{t("diag.table.process")}</th>
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
                  {t("diag.table.empty")}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="text-[10px] text-slate-500">
        {t("diag.log_hint")}
      </p>
    </div>
  );
}

/** Tarjeta de un subsistema interno: estado (punto de color) + detalle. */
function SubsystemCard({
  icon,
  title,
  ok,
  status,
  detail,
  warnWhenOk = false,
}: {
  icon: React.ReactNode;
  title: string;
  ok: boolean;
  status: string;
  detail: string;
  /** Para el grabador: "activo" es amarillo (consume), no verde. */
  warnWhenOk?: boolean;
}) {
  const dot = ok
    ? warnWhenOk
      ? "bg-amber-400"
      : "bg-emerald-400"
    : "bg-slate-600";
  const statusColor = ok
    ? warnWhenOk
      ? "text-amber-300"
      : "text-emerald-300"
    : "text-slate-500";
  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-2.5 py-2">
      <div className="flex items-center justify-between">
        <span className="flex items-center gap-1.5 text-[11px] font-semibold text-slate-200">
          <span className="text-slate-400">{icon}</span>
          {title}
        </span>
        <span className={`flex items-center gap-1 text-[10px] font-medium ${statusColor}`}>
          <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
          {status}
        </span>
      </div>
      <p className="mt-1 truncate text-[10px] text-slate-500">{detail}</p>
    </div>
  );
}
