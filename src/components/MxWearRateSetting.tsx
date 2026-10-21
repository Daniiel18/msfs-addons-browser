import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useTechLogStore } from "../stores/useTechLogStore";
import { useToastStore } from "../stores/useToastStore";

/** (19/10) Etapa 3B — ritmo de desgaste del Tech Log de SimFleet. La cabina se
 *  gasta a 2/3 del ritmo de las piezas (igual que los valores por defecto de
 *  TechLog: ×3 / ×2). Cambiarlo no hace saltar el % de las piezas. */
const OPTIONS = [1, 2, 3, 5] as const;

export function MxWearRateSetting() {
  const [rate, setRate] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const pushToast = useToastStore((s) => s.push);
  useEffect(() => {
    api
      .mxGetWearRates()
      .then(([life]) => setRate(life))
      .catch(() => setRate(3));
  }, []);
  const choose = async (r: number) => {
    if (r === rate || busy) return;
    setBusy(true);
    try {
      await api.mxSetWearRates(r, Math.max(0.5, Math.round(((r * 2) / 3) * 10) / 10));
      setRate(r);
      useTechLogStore.setState((s) => ({ epoch: s.epoch + 1 }));
      pushToast({ kind: "success", title: t("settings.mx.saved", { r: String(r) }), ttlMs: 2500 });
    } catch (e) {
      pushToast({ kind: "error", title: t("settings.mx.error"), message: String(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="rounded-md border border-sf-border bg-sf-surface px-3 py-2.5">
      <div className="flex items-center gap-2 text-xs text-sf-text">
        {t("settings.mx.rate_title")}
        {busy && <Loader2 className="h-3 w-3 animate-spin text-sf-muted" />}
      </div>
      <p className="mt-0.5 text-[11px] text-sf-muted">{t("settings.mx.rate_hint")}</p>
      <div className="mt-2 grid grid-cols-2 gap-1.5 sm:grid-cols-4">
        {OPTIONS.map((r) => (
          <button
            key={r}
            onClick={() => void choose(r)}
            disabled={rate === null || busy}
            className={cn(
              "rounded-md border px-2 py-1.5 text-left text-[11px] transition-colors disabled:opacity-60",
              rate === r
                ? "border-brand-500/60 bg-brand-500/15 text-brand-200"
                : "border-sf-border bg-sf-raised text-sf-text hover:border-brand-500/40",
            )}
          >
            <div className="font-semibold">×{r}</div>
            <div className="text-[10px] text-sf-muted">{t(`settings.mx.rate_${r}`)}</div>
          </button>
        ))}
      </div>
      <p className="mt-2 text-[10px] text-sf-muted">{t("settings.mx.techlog_note")}</p>
    </div>
  );
}
