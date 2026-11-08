import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, Loader2, RefreshCcw, Trash2 } from "lucide-react";
import { api } from "../lib/tauri";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import type { CleanedCache, GpuInfo, OptimizerReport, SimSettings, WindowsTip } from "../lib/types";
import { formatStorage } from "./AddonsView";

type Tone = "ok" | "advice" | "info";

const TIP_TONE: Record<string, Tone> = {
  "game_mode.on": "ok",
  "game_mode.off": "advice",
  "background_recording.on": "advice",
  "background_recording.off": "ok",
  "power_plan.high": "ok",
  "power_plan.ultimate": "ok",
  "power_plan.balanced": "advice",
  "power_plan.saver": "advice",
};
const TONE_DOT: Record<Tone, string> = { ok: "bg-emerald-400", advice: "bg-amber-400", info: "bg-sf-muted" };

function driverLine(gpu: GpuInfo): string {
  const date = gpu.driverDate
    ? new Date(`${gpu.driverDate}T12:00:00Z`).toLocaleDateString(getActiveLocale(), { day: "numeric", month: "short", year: "numeric" })
    : null;
  return [gpu.driverVersion && t("optimizer.driver", { v: gpu.driverVersion }), date].filter(Boolean).join(" · ");
}

function simLine(s: SimSettings): string {
  const parts = [
    s.preset && t("optimizer.sim.preset", { v: s.preset }),
    s.antiAliasing && [s.antiAliasing, s.upscalingMode].filter(Boolean).join(" "),
    s.frameGeneration &&
      (s.frameGeneration === "NONE" ? t("optimizer.sim.no_fg") : t("optimizer.sim.fg", { v: s.frameGeneration })),
    s.frameLimiter && (s.frameLimiter === "0" ? t("optimizer.sim.no_limit") : t("optimizer.sim.limit", { v: s.frameLimiter })),
    s.vsync !== null && t(s.vsync ? "optimizer.sim.vsync_on" : "optimizer.sim.vsync_off"),
  ];
  return parts.filter(Boolean).join(" · ");
}

function Tip({ tip }: { tip: WindowsTip }) {
  const tone = TIP_TONE[`${tip.id}.${tip.value}`] ?? "info";
  return (
    <li className="flex items-start gap-2 text-xs">
      <span className={cn("mt-1 h-2 w-2 shrink-0 rounded-full", TONE_DOT[tone])} />
      <span className="min-w-0">
        <span className="font-medium text-sf-text">{t(`optimizer.tip.${tip.id}`)}: </span>
        <span className="text-sf-muted">{t(`optimizer.tip.${tip.id}.${tip.value}`)}</span>
      </span>
    </li>
  );
}

/** (06/11) Revisa lo que afecta a MSFS y vacía solo las cachés que marques. */
export function OptimizerPanel() {
  const [report, setReport] = useState<OptimizerReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [marked, setMarked] = useState<Set<string>>(new Set());
  const [sure, setSure] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<CleanedCache[]>([]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setReport(await api.optimizerReport());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const toggle = (id: string) => {
    setSure(false);
    setMarked((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  };
  const clean = async () => {
    setBusy(true);
    setError(null);
    try {
      setDone(await api.optimizerClean([...marked]));
      setMarked(new Set());
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setSure(false);
    }
  };

  if (!report) {
    return (
      <div className="flex items-center gap-2 rounded-xl border border-sf-border bg-sf-surface px-4 py-3 text-xs text-sf-muted">
        {loading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
        {error ?? t("optimizer.loading")}
      </div>
    );
  }

  const selected = report.caches.filter((c) => marked.has(c.id));
  const selectedBytes = selected.reduce((sum, c) => sum + c.sizeBytes, 0);
  const label = (id: string) => t(`optimizer.cache.${id}`);

  return (
    <div className="space-y-3 rounded-xl border border-sf-border bg-sf-surface px-4 py-3 text-xs">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-0.5">
          {report.gpus.length === 0 && <p className="text-sf-muted">{t("optimizer.gpu_none")}</p>}
          {report.gpus.map((gpu, i) => (
            <p key={gpu.name} className={i === 0 ? "text-sm font-semibold text-sf-text" : "text-sf-muted"}>
              {gpu.name}
              <span className="ml-2 text-[11px] font-normal text-sf-muted">{driverLine(gpu)}</span>
            </p>
          ))}
        </div>
        <button
          onClick={() => void load()}
          disabled={loading}
          title={t("optimizer.refresh")}
          aria-label={t("optimizer.refresh")}
          className="shrink-0 rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text disabled:opacity-50"
        >
          <RefreshCcw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
        </button>
      </div>

      {report.sim?.otherAdapter && (
        <p className="flex items-start gap-1.5 text-amber-300">
          <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
          {t("optimizer.other_adapter", { name: report.sim.adapter ?? "" })}
        </p>
      )}
      {report.sim && (
        <p className="text-sf-muted">
          <span className="font-medium text-sf-text">{t("optimizer.sim")}: </span>
          {simLine(report.sim)}
        </p>
      )}

      <div className="border-t border-sf-border pt-2">
        <p className="font-medium text-sf-text">{t("optimizer.caches")}</p>
        <p className="mt-0.5 text-[11px] text-sf-muted">{t("optimizer.caches_hint")}</p>
        {report.caches.length === 0 ? (
          <p className="mt-2 text-sf-muted">{t("optimizer.caches_none")}</p>
        ) : (
          <ul className="mt-2 space-y-1">
            {report.caches.map((cache) => (
              <li key={cache.id}>
                <label className="flex cursor-pointer items-center gap-2" title={cache.path}>
                  <input
                    type="checkbox"
                    checked={marked.has(cache.id)}
                    onChange={() => toggle(cache.id)}
                    className="h-3.5 w-3.5 accent-brand-500"
                  />
                  <span className="min-w-0 flex-1 truncate text-sf-text">{label(cache.id)}</span>
                  <span className="shrink-0 font-mono text-[11px] text-sf-muted">{formatStorage(cache.sizeBytes)}</span>
                </label>
                {cache.id === "rolling" && (
                  <p className="ml-[1.375rem] text-[11px] leading-snug text-sf-muted">{t("optimizer.rolling_note")}</p>
                )}
              </li>
            ))}
          </ul>
        )}
        {report.caches.length > 0 && (
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {sure ? (
              <>
                <span className="min-w-0 flex-1 text-[11px] leading-snug text-sf-muted">{t("optimizer.confirm")}</span>
                <button onClick={() => setSure(false)} className="rounded-lg border border-sf-border px-2.5 py-1 font-medium text-sf-text hover:bg-sf-raised">
                  {t("optimizer.cancel")}
                </button>
                <button
                  onClick={() => void clean()}
                  disabled={busy}
                  className="inline-flex items-center gap-1 rounded-lg bg-rose-500/90 px-2.5 py-1 font-semibold text-white hover:bg-rose-500 disabled:opacity-50"
                >
                  {busy ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
                  {t("optimizer.confirm_yes")}
                </button>
              </>
            ) : (
              <>
                <span className="min-w-0 flex-1 text-[11px] text-sf-muted">
                  {report.simRunning ? t("optimizer.sim_running") : selected.length > 0 && t("optimizer.selected", { size: formatStorage(selectedBytes) })}
                </span>
                <button
                  onClick={() => setSure(true)}
                  disabled={selected.length === 0 || report.simRunning}
                  className="inline-flex items-center gap-1 rounded-lg border border-sf-border bg-sf-raised px-2.5 py-1 font-semibold text-sf-text hover:bg-sf-surface disabled:opacity-50"
                >
                  <Trash2 className="h-3 w-3" />
                  {t("optimizer.clean", { n: selected.length })}
                </button>
              </>
            )}
          </div>
        )}
        {error && <p className="mt-2 text-[11px] text-rose-300">{error}</p>}
        {done.length > 0 && (
          <ul className="mt-2 space-y-0.5 rounded-lg bg-sf-raised px-2.5 py-1.5 text-[11px] text-sf-text">
            {done.map((item) => (
              <li key={item.id}>
                {t("optimizer.result", {
                  name: label(item.id),
                  before: formatStorage(item.beforeBytes),
                  after: formatStorage(item.afterBytes),
                })}
                {item.skippedFiles > 0 && ` · ${t("optimizer.skipped", { n: item.skippedFiles })}`}
              </li>
            ))}
          </ul>
        )}
      </div>

      {report.windows.length > 0 && (
        <div className="border-t border-sf-border pt-2">
          <p className="font-medium text-sf-text">{t("optimizer.windows")}</p>
          <p className="mt-0.5 text-[11px] text-sf-muted">{t("optimizer.windows_hint")}</p>
          <ul className="mt-2 space-y-1">
            {report.windows.map((tip) => (
              <Tip key={tip.id} tip={tip} />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
