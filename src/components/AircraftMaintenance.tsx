import { useCallback, useEffect, useMemo, useState } from "react";
import { X, Wrench, Plane, Loader2 } from "lucide-react";
import type { AircraftMaint, AirlineLedger, AirlinePolicy } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { AirlineLogo } from "./AirlineLogo";
import { TechLogSummaryCard, useTechLogAircraft } from "./TechLogPanel";
import { useTechLogStore } from "../stores/useTechLogStore";
import { useAppStore } from "../stores/useAppStore";

const MAINT_LEVELS = [0, 1, 2] as const;

/** Normaliza una matrícula para comparar ("SE-RSC" → "SERSC"). */
const normReg = (s: string) => s.replace(/[^a-z0-9]/gi, "").toUpperCase();

/**
 * (20/10) Etapa 4 — Mantenimiento de la aerolínea en Finanzas.
 *
 * El desgaste por componentes del sistema viejo ya no existe: cada avión lleva
 * su Tech Log (Fenix/PMDG TechLog o el de SimFleet) y se repara en el Hangar.
 * Aquí queda el NIVEL de mantenimiento de la aerolínea (precio de cada trabajo
 * y mantenimiento nocturno) y un resumen por avión con acceso al Hangar.
 */
export function MaintenanceModal({
  led,
  initialReg,
  onClose,
  onChanged,
}: {
  led: AirlineLedger;
  initialReg?: string | null;
  onClose: () => void;
  onChanged: () => void | Promise<void>;
}) {
  const [fleet, setFleet] = useState<AircraftMaint[] | null>(null);
  const [policy, setPolicy] = useState<AirlinePolicy | null>(null);
  const [selected, setSelected] = useState<string | null>(initialReg ?? null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const [f, p] = await Promise.all([
      api.airlineFleetMaintenance(led.key),
      api.airlinePolicy(led.key),
    ]);
    setFleet(f);
    setPolicy(p);
    // Preselecciona la matrícula del Hangar (compara normalizada: "SE-RSC"
    // del Hangar vs la que devuelve la flota, con o sin guion).
    setSelected((cur) => {
      if (initialReg) {
        const m = f.find((a) => normReg(a.registration) === normReg(initialReg));
        if (m) return m.registration;
      }
      return cur ?? f[0]?.registration ?? null;
    });
  }, [led.key, initialReg]);

  useEffect(() => {
    void load();
  }, [load]);

  const current = useMemo(
    () => fleet?.find((a) => a.registration === selected) ?? fleet?.[0] ?? null,
    [fleet, selected],
  );
  const managed = useTechLogStore((s) => s.isManaged(current?.registration));
  const techlog = useTechLogAircraft(current?.registration, managed, current?.model);

  const setLevel = async (level: number) => {
    if (!policy) return;
    setBusy(true);
    try {
      const next = { ...policy, maintenanceLevel: level };
      await api.setAirlinePolicy(led.key, next);
      setPolicy(next);
      await onChanged();
    } finally {
      setBusy(false);
    }
  };

  const openHangar = () => {
    if (!current) return;
    useAppStore.getState().requestHangarFocus(current.registration);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <div
        className="flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* header */}
        <div className="flex items-center gap-3 border-b border-sf-border p-4">
          <AirlineLogo icao={led.icao} name={led.name} size={32} />
          <div className="min-w-0 flex-1">
            <h3 className="flex items-center gap-1.5 truncate text-sm font-semibold text-sf-text">
              <Wrench className="h-4 w-4 text-amber-400" />
              {t("economy.maint.title")}
            </h3>
            <p className="truncate text-xs text-sf-muted">{led.name}</p>
          </div>
          <button onClick={onClose} className="rounded-lg p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text/90">
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* maintenance level */}
        <div className="border-b border-sf-border px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs text-sf-muted">{t("economy.maint.level")}:</span>
            <div className="flex items-center gap-1 rounded-lg bg-slate-950/70 p-0.5 ring-1 ring-sf-border">
              {MAINT_LEVELS.map((lvl) => {
                const name = ["basic", "standard", "premium"][lvl];
                const on = policy?.maintenanceLevel === lvl;
                return (
                  <button
                    key={lvl}
                    disabled={busy}
                    onClick={() => setLevel(lvl)}
                    className={`rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors disabled:opacity-50 ${
                      on ? "bg-brand-500/20 text-brand-200" : "text-sf-muted hover:text-sf-text"
                    }`}
                  >
                    {t(`economy.maint.${name}`)}
                  </button>
                );
              })}
            </div>
          </div>
          <p className="mt-1.5 text-[11px] text-sf-muted">{t("economy.maint.level_hint")}</p>
        </div>

        {!fleet ? (
          <div className="py-20 text-center text-sm text-sf-muted">{t("economy.maint.loading")}</div>
        ) : fleet.length === 0 ? (
          <div className="py-20 text-center text-sm text-sf-muted">{t("economy.maint.empty")}</div>
        ) : (
          <div className="flex min-h-0 flex-1 overflow-hidden">
            {/* fleet list */}
            <div className="w-44 shrink-0 overflow-y-auto border-r border-sf-border p-2">
              {fleet.map((a) => {
                const on = a.registration === current?.registration;
                return (
                  <button
                    key={a.registration}
                    onClick={() => setSelected(a.registration)}
                    className={`mb-1.5 w-full rounded-xl border p-2 text-left transition-colors ${
                      on ? "border-brand-500/60 bg-brand-500/10" : "border-transparent bg-sf-bg/40 hover:bg-sf-raised/70"
                    }`}
                  >
                    <div className="flex items-center gap-1.5">
                      <Plane className="h-3 w-3 text-sf-muted" />
                      <span className="truncate text-xs font-semibold text-sf-text">{a.registration}</span>
                    </div>
                    <div className="truncate text-[11px] text-sf-muted">{a.model ?? "—"}</div>
                  </button>
                );
              })}
            </div>

            {/* detail: resumen del Tech Log del avión */}
            <div className="min-w-0 flex-1 overflow-y-auto p-4">
              {!current ? (
                <p className="py-10 text-center text-sm text-sf-muted">{t("economy.maint.select")}</p>
              ) : (
                <div className="space-y-3">
                  <div className="flex items-baseline justify-between">
                    <h4 className="text-sm font-semibold text-sf-text">{current.registration}</h4>
                    <span className="text-[11px] text-sf-muted">
                      {t("economy.maint.flights", { n: current.flights })}
                    </span>
                  </div>
                  {techlog.data ? (
                    <TechLogSummaryCard data={techlog.data} onOpen={openHangar} />
                  ) : (
                    <>
                      <div className="flex items-center justify-center gap-2 rounded-xl border border-sf-border bg-sf-bg/40 py-10 text-xs text-sf-muted">
                        {techlog.loading && <Loader2 className="h-4 w-4 animate-spin" />}
                        {t(techlog.loading ? "hangar.techlog_loading" : "hangar.techlog_unavailable")}
                      </div>
                      <button
                        onClick={openHangar}
                        className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-amber-500/30 bg-amber-500/10 py-2 text-xs font-semibold text-amber-300 transition-colors hover:bg-amber-500/20"
                      >
                        <Wrench className="h-3.5 w-3.5" />
                        {t("economy.maint.open_hangar")}
                      </button>
                    </>
                  )}
                  <p className="text-[11px] text-sf-muted">{t("economy.maint.hint_techlog")}</p>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}