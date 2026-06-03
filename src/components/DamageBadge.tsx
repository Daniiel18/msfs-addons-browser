import { useEffect, useState } from "react";
import {
  AlertTriangle,
  HelpCircle,
  Loader2,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import type { DamageReport } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (v3.26.0 — P7.10) Chip con el veredicto de daños del vuelo:
 * LIMPIO / FORZADO / DAÑADO. Se deriva de los avisos de sobrevelocidad,
 * pérdida (stall), presión de aceite y la fuerza-G capturados por
 * SimConnect durante el vuelo. Para vuelos viejos / importados que no
 * tienen estos datos muestra "Sin datos" (no penaliza).
 */
export function DamageBadge({ flightId }: { flightId: number }) {
  const [report, setReport] = useState<DamageReport | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setReport(null);
    api
      .analyzeFlightDamage(flightId)
      .then((r) => {
        if (!cancelled) setReport(r);
      })
      .catch(() => {
        if (!cancelled) setReport(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [flightId]);

  if (loading) {
    return (
      <div className="flex items-center gap-1.5 rounded-md bg-slate-800/60 px-2 py-1 text-[11px] text-slate-400 ring-1 ring-slate-700/60">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        {t("fb.damage.analyzing")}
      </div>
    );
  }
  if (!report) return null;

  const v = report.verdict;
  const cfg =
    v === "clean"
      ? {
          Icon: ShieldCheck,
          cls: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
          label: t("fb.damage.verdict.clean"),
        }
      : v === "forced"
        ? {
            Icon: AlertTriangle,
            cls: "bg-amber-500/15 text-amber-300 ring-amber-500/30",
            label: t("fb.damage.verdict.forced"),
          }
        : v === "damaged"
          ? {
              Icon: ShieldAlert,
              cls: "bg-rose-500/15 text-rose-300 ring-rose-500/30",
              label: t("fb.damage.verdict.damaged"),
            }
          : {
              Icon: HelpCircle,
              cls: "bg-slate-700/40 text-slate-400 ring-slate-600/40",
              label: t("fb.damage.verdict.no_data"),
            };
  const Icon = cfg.Icon;

  const reasons = report.reasons.map((k) => t(k)).filter(Boolean);
  const tip =
    v === "no_data"
      ? t("fb.damage.no_data_hint")
      : reasons.length
        ? reasons.join(" · ")
        : t("fb.damage.clean_hint");

  return (
    <div
      title={tip}
      className={`flex w-fit items-center gap-1.5 rounded-md px-2 py-1 text-[11px] font-medium ring-1 ${cfg.cls}`}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" />
      <span>
        {t("fb.damage.label")}: {cfg.label}
      </span>
      {report.hasData && report.maxG != null && (
        <span className="ml-1 font-mono text-[10px] opacity-80">
          {report.maxG.toFixed(1)}g
        </span>
      )}
    </div>
  );
}
