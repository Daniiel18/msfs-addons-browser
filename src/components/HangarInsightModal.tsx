import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowRight, Loader2, MapPin, Plane, Wallet, X } from "lucide-react";
import type { HangarCount, HangarInsight, HangarInsightFilter } from "../lib/types";
import { api } from "../lib/tauri";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useAppStore } from "../stores/useAppStore";
import { AirlineLogo } from "./AirlineLogo";

/**
 * (26/10) Etapa 8 — Ficha de un elemento de los paneles del Hangar: un tipo
 * de avión, una aerolínea, un destino o un país. Sus cifras, el desglose y
 * sus últimos vuelos (cada uno abre el FlightBook en ese vuelo; cada avión,
 * su ficha del Hangar). Aerolínea → "Ver finanzas"; destino → "Ver en el
 * mapa" (la tarjeta del aeropuerto en el FlightBook).
 */
export type InsightKind = "type" | "airline" | "destination" | "region";

export interface InsightTarget {
  kind: InsightKind;
  filter: HangarInsightFilter;
  title: string;
  /** Logo o bandera. */
  img?: string | null;
  /** ICAO de la aerolínea (logo y Finanzas) o del aeropuerto. */
  code?: string | null;
}

export function HangarInsightModal({
  target,
  onClose,
  onTarget,
  onSelectAircraft,
}: {
  target: InsightTarget | null;
  onClose: () => void;
  /** Cambia la ficha (p. ej. un aeropuerto dentro de un país). */
  onTarget: (t: InsightTarget) => void;
  /** Abre la ficha de ese avión en el Hangar. */
  onSelectAircraft: (key: string) => void;
}) {
  const [data, setData] = useState<HangarInsight | null>(null);
  const [error, setError] = useState<string | null>(null);
  const filterKey = target ? JSON.stringify(target.filter) : "";
  useEffect(() => {
    if (!target) return;
    let alive = true;
    setData(null);
    setError(null);
    api
      .hangarInsight(target.filter)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
    // `filterKey` resume `target.filter`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filterKey]);
  useEffect(() => {
    if (!target) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [target, onClose]);

  const go = (f: () => void) => {
    f();
    onClose();
  };
  const openFlight = (id: number) => go(() => useAppStore.getState().requestFlightFocus(id));

  const airportRow = (c: HangarCount) => (
    <BreakRow
      key={c.key ?? c.label}
      lead={<span className="w-11 shrink-0 font-mono text-[11px] font-semibold text-brand-300">{c.code}</span>}
      label={c.label}
      count={c.count}
      onClick={
        c.code && !(target?.kind === "destination" && target.code === c.code)
          ? () =>
              onTarget({
                kind: "destination",
                filter: { kind: "airports", icaos: [c.code!] },
                title: c.code!,
                code: c.code,
                img: target?.kind === "region" ? target.img : null,
              })
          : undefined
      }
    />
  );
  const airlineRow = (c: HangarCount) => (
    <BreakRow
      key={c.key ?? c.label}
      lead={<AirlineLogo icao={c.code} name={c.label} size={18} />}
      label={c.label}
      count={c.count}
      onClick={
        target?.kind !== "airline" && c.key
          ? () => onTarget({ kind: "airline", filter: { kind: "airline", key: c.key }, title: c.label, code: c.code })
          : undefined
      }
    />
  );
  const typeRow = (c: HangarCount) => (
    <BreakRow
      key={c.key ?? c.label}
      lead={<Plane className="h-3.5 w-3.5 shrink-0 text-sf-muted" />}
      label={c.label}
      count={c.count}
      onClick={
        target?.kind !== "type" && c.key
          ? () => onTarget({ kind: "type", filter: { kind: "type", key: c.key }, title: c.label })
          : undefined
      }
    />
  );
  const aircraftRows = (d: HangarInsight) =>
    d.aircraft.map((a) => (
      <BreakRow
        key={a.key}
        lead={<span className="w-16 shrink-0 truncate font-mono text-[11px] font-semibold text-sf-text">{a.registration ?? "—"}</span>}
        label={a.model ?? ""}
        count={a.flights}
        onClick={a.key !== "unknown" ? () => go(() => onSelectAircraft(a.key)) : undefined}
      />
    ));

  const sections = (d: HangarInsight): { title: string; rows: ReactNode[]; max: number }[] => {
    const s = (title: string, list: HangarCount[], row: (c: HangarCount) => ReactNode) => ({
      title,
      rows: list.map(row),
      max: Math.max(1, ...list.map((c) => c.count)),
    });
    const ac = { title: t("hinsight.sec.aircraft"), rows: aircraftRows(d), max: Math.max(1, ...d.aircraft.map((a) => a.flights)) };
    switch (target?.kind) {
      case "type":
        return [s(t("hinsight.sec.airlines"), d.airlines, airlineRow), s(t("hinsight.sec.destinations"), d.destinations, airportRow), ac];
      case "airline":
        return [s(t("hinsight.sec.types"), d.types, typeRow), s(t("hinsight.sec.destinations"), d.destinations, airportRow), ac];
      case "destination":
        return [s(t("hinsight.sec.airlines"), d.airlines, airlineRow), s(t("hinsight.sec.types"), d.types, typeRow), s(t("hinsight.sec.origins"), d.origins, airportRow)];
      default:
        return [s(t("hinsight.sec.airports"), d.destinations, airportRow), s(t("hinsight.sec.airlines"), d.airlines, airlineRow), s(t("hinsight.sec.types"), d.types, typeRow)];
    }
  };

  return createPortal(
    <AnimatePresence>
      {target && (
        <motion.div
          key="hinsight"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="fixed inset-x-0 bottom-0 top-11 z-[60] flex items-center justify-center bg-slate-950/70 p-4"
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            initial={{ y: 8, scale: 0.98 }}
            animate={{ y: 0, scale: 1 }}
            exit={{ y: 8, scale: 0.98 }}
            transition={{ duration: 0.14 }}
            data-sf-overlay
            className="flex max-h-full w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl"
          >
            <header className="flex items-center gap-3 border-b border-sf-border px-5 py-4">
              {target.kind === "airline" ? (
                <AirlineLogo icao={target.code} name={target.title} size={36} />
              ) : target.img ? (
                <img src={target.img} alt="" className="h-9 w-12 shrink-0 rounded object-cover ring-1 ring-sf-border" />
              ) : (
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-brand-500/15 text-brand-300 ring-1 ring-brand-500/25">
                  {target.kind === "type" ? <Plane className="h-4 w-4" /> : <MapPin className="h-4 w-4" />}
                </span>
              )}
              <div className="min-w-0 flex-1">
                <div className="text-[11px] uppercase tracking-wide text-sf-muted">{t(`hinsight.kind.${target.kind}`)}</div>
                <h2 className="truncate text-base font-semibold text-sf-text">{target.title}</h2>
              </div>
              <button onClick={onClose} title={t("common.close")} className="rounded-lg p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text">
                <X className="h-4 w-4" />
              </button>
            </header>

            <div className="flex-1 space-y-4 overflow-y-auto px-5 py-4">
              {error && <p className="text-xs text-rose-300">{error}</p>}
              {!data && !error && (
                <div className="flex justify-center py-12 text-sf-muted">
                  <Loader2 className="h-5 w-5 animate-spin" />
                </div>
              )}
              {data && data.flights === 0 && <p className="py-8 text-center text-xs text-sf-muted">{t("hinsight.none")}</p>}
              {data && data.flights > 0 && (
                <>
                  <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                    <Kpi label={t("hinsight.kpi.flights")} value={fmt(data.flights)} />
                    <Kpi label={t("hinsight.kpi.hours")} value={`${fmt(Math.round(data.totalTimeS / 360) / 10, 1)} h`} />
                    <Kpi label={t("hinsight.kpi.miles")} value={`${fmt(data.totalNm)} nm`} />
                    <Kpi label={t("hinsight.kpi.pax")} value={data.passengers ? fmt(data.passengers) : "—"} />
                    <Kpi
                      label={t("hinsight.kpi.avg_fpm")}
                      value={data.avgFpm != null ? `${fmt(data.avgFpm)} fpm` : "—"}
                      tone={fpmTone(data.avgFpm)}
                    />
                    <Kpi
                      label={t("hinsight.kpi.best")}
                      value={data.bestFpm != null ? `${fmt(data.bestFpm)} fpm` : "—"}
                      tone={fpmTone(data.bestFpm)}
                      onClick={data.bestFlightId != null ? () => openFlight(data.bestFlightId!) : undefined}
                      hint={t("hinsight.open_flight")}
                    />
                  </div>
                  {data.firstAt && data.lastAt && (
                    <p className="text-[11px] text-sf-muted">
                      {t("hinsight.range", { from: fmtDay(data.firstAt), to: fmtDay(data.lastAt) })}
                    </p>
                  )}

                  <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                    {sections(data).map((sec) => (
                      <section key={sec.title} className="rounded-xl border border-sf-border bg-sf-surface p-3">
                        <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{sec.title}</h3>
                        {sec.rows.length === 0 ? (
                          <p className="text-[11px] text-sf-muted">—</p>
                        ) : (
                          <BarScale.Provider value={sec.max}>
                            <ul className="space-y-1">{sec.rows}</ul>
                          </BarScale.Provider>
                        )}
                      </section>
                    ))}
                  </div>

                  <section>
                    <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("hinsight.recent")}</h3>
                    <ul className="divide-y divide-sf-border/60 overflow-hidden rounded-xl border border-sf-border bg-sf-surface">
                      {data.recent.map((f) => (
                        <li key={f.id}>
                          <button
                            onClick={() => openFlight(f.id)}
                            title={t("hinsight.open_flight")}
                            className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs hover:bg-sf-raised/60"
                          >
                            <span className="w-20 shrink-0 text-sf-muted">{fmtDay(f.startedAt)}</span>
                            <span className="w-28 shrink-0 font-mono font-semibold text-sf-text">
                              {f.origin ?? "?"} → {f.destination ?? "?"}
                            </span>
                            <span className="min-w-0 flex-1 truncate text-sf-muted">
                              {[f.label, f.registration, f.model].filter(Boolean).join(" · ")}
                            </span>
                            {f.fpm != null && (
                              <span className={cn("shrink-0 font-mono tabular-nums", FPM_TEXT[fpmTone(f.fpm)])}>{f.fpm} fpm</span>
                            )}
                            <ArrowRight className="h-3.5 w-3.5 shrink-0 text-sf-muted" />
                          </button>
                        </li>
                      ))}
                    </ul>
                  </section>
                </>
              )}
            </div>

            {(target.kind === "airline" || target.kind === "destination") && (
              <footer className="flex justify-end gap-2 border-t border-sf-border px-5 py-3">
                {target.kind === "airline" && (
                  <button
                    onClick={() => go(() => useAppStore.getState().openEconomy(target.code ?? target.title))}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400"
                  >
                    <Wallet className="h-3.5 w-3.5" />
                    {t("hinsight.open_finance")}
                  </button>
                )}
                {target.kind === "destination" && target.code && (
                  <button
                    onClick={() => go(() => useAppStore.getState().requestAirportCard(target.code!, null))}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400"
                  >
                    <MapPin className="h-3.5 w-3.5" />
                    {t("hinsight.open_map")}
                  </button>
                )}
              </footer>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

// ---------------------------------------------------------------------------

/** Máximo de la sección, para escalar las barras. */
const BarScale = createContext(1);

function BreakRow({
  lead,
  label,
  count,
  onClick,
}: {
  lead: ReactNode;
  label: string;
  count: number;
  onClick?: () => void;
}) {
  const max = useContext(BarScale);
  const inner = (
    <>
      <span className="flex min-w-0 flex-1 items-center gap-2">
        {lead}
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs text-sf-text">{label}</span>
          <span className="mt-0.5 block h-1 overflow-hidden rounded-full bg-sf-raised">
            <span className="block h-full rounded-full bg-brand-400/70" style={{ width: `${(count / max) * 100}%` }} />
          </span>
        </span>
      </span>
      <span className="w-6 shrink-0 text-right text-[11px] tabular-nums text-sf-muted">{count}</span>
    </>
  );
  return (
    <li>
      {onClick ? (
        <button onClick={onClick} className="flex w-full items-center gap-2 rounded-md px-1 py-1 text-left hover:bg-sf-raised/60">
          {inner}
        </button>
      ) : (
        <div className="flex items-center gap-2 px-1 py-1">{inner}</div>
      )}
    </li>
  );
}

type FpmTone = "good" | "ok" | "hard" | "none";
const FPM_TEXT: Record<FpmTone, string> = {
  good: "text-emerald-300",
  ok: "text-sky-300",
  hard: "text-amber-300",
  none: "text-sf-text",
};
function fpmTone(fpm: number | null | undefined): FpmTone {
  if (fpm == null) return "none";
  const v = -Math.abs(fpm);
  return v > -150 ? "good" : v > -300 ? "ok" : "hard";
}

function Kpi({
  label,
  value,
  tone = "none",
  onClick,
  hint,
}: {
  label: string;
  value: string;
  tone?: FpmTone;
  onClick?: () => void;
  hint?: string;
}) {
  const body = (
    <>
      <div className="text-[10px] uppercase tracking-wide text-sf-muted">{label}</div>
      <div className={cn("font-mono text-base font-semibold", FPM_TEXT[tone])}>{value}</div>
    </>
  );
  return onClick ? (
    <button
      onClick={onClick}
      title={hint}
      className="rounded-xl border border-sf-border bg-sf-surface px-3 py-2 text-left hover:border-brand-500/40"
    >
      {body}
    </button>
  ) : (
    <div className="rounded-xl border border-sf-border bg-sf-surface px-3 py-2">{body}</div>
  );
}

function fmt(n: number, d = 0): string {
  return n.toLocaleString(getActiveLocale(), { minimumFractionDigits: d, maximumFractionDigits: d });
}

function fmtDay(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString(getActiveLocale(), { day: "numeric", month: "short", year: "numeric" });
}
