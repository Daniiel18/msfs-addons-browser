import { useCallback, useEffect, useMemo, useState } from "react";
import { Armchair, CheckCircle2, ChevronRight, Loader2, Wrench, X } from "lucide-react";
import type { TechLogAircraft, TechLogCabinPage, TechLogCabinPlan, TechLogFitting, TechLogSeat } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useTechLogStore } from "../stores/useTechLogStore";
import { barColor, CabinSection, Empty, fmtDate, Section, useTechLogOrders } from "./TechLogPanel";
import { TechLogPageNotice } from "./TechLogTabs";
import { DetailModal } from "./DetailModal";

/**
 * (01/11) Etapa 5a — página «Cabin» de Fenix/PMDG TechLog en SimFleet:
 * plano de asientos coloreado por la vida que les queda, galleys, baños y
 * sistemas de cabina con sus barras de vida y sus órdenes (reparar /
 * reemplazar, las mismas que las piezas, cobradas en Finanzas). «Sistemas»
 * es la cabina en vivo del Fenix que antes estaba en Estado. El plano es un
 * dibujo propio de SimFleet.
 */

type SeatTone = "good" | "worn" | "low" | "end" | "defect" | "blocked";

function seatTone(s: TechLogSeat): SeatTone {
  if (s.blocked) return "blocked";
  if (s.broken.length > 0) return "defect";
  const p = s.worstRemainingPct ?? 100;
  return p < 10 ? "end" : p < 30 ? "low" : p < 60 ? "worn" : "good";
}

const SEAT_FILL: Record<SeatTone, string> = {
  good: "#34d399",
  worn: "#a3e635",
  low: "#fbbf24",
  end: "#f87171",
  defect: "#f43f5e",
  blocked: "#52525b",
};

type Spot = { type: "galley" | "lav" | "crew" | "zone" | "other"; end: "FWD" | "MID" | "AFT" | null };

/** Dónde va cada equipo en el plano (por su clave y su posición). */
function spotOf(f: TechLogFitting): Spot {
  const k = f.key.toUpperCase();
  const type = /^(GALLEY|OVEN|COFFEE|WATER_BOILER|CHILLER|BAR_TROLLEY|TROLLEY)/.test(k)
    ? "galley"
    : /^LAV/.test(k)
      ? "lav"
      : /ATTENDANT|CREW_SEAT/.test(k)
        ? "crew"
        : /CABIN_LIGHT|PA_SPEAKER|PA_/.test(k)
          ? "zone"
          : "other";
  const pos = (f.position ?? "").toUpperCase();
  const end = pos.startsWith("FWD") ? "FWD" : pos.startsWith("MID") ? "MID" : pos.startsWith("AFT") ? "AFT" : null;
  return { type, end };
}

type Selection = { kind: "seat"; seat: string } | { kind: "spot"; type: Spot["type"]; end: Spot["end"] } | null;

export function TechLogCabin({
  data,
  onReload,
  refreshKey,
}: {
  data: TechLogAircraft;
  onReload: () => void;
  refreshKey: number;
}) {
  const epoch = useTechLogStore((s) => s.epoch);
  const [view, setView] = useState<"plan" | "systems">("plan");
  const [page, setPage] = useState<TechLogCabinPage | null>(null);
  const [sel, setSel] = useState<Selection>(null);

  const load = useCallback(async () => {
    try {
      setPage(await api.techlogCabinPlan(data.registration));
    } catch {
      setPage({ state: "unsupported", source: data.source, activeRegistration: null, savedAt: null, plan: null });
    }
  }, [data.registration, data.source]);
  useEffect(() => {
    void load();
  }, [load, epoch, refreshKey, data.running, data.active]);
  const live = page?.state === "live";
  useEffect(() => {
    const id = window.setInterval(() => void load(), live ? 15_000 : 30_000);
    return () => window.clearInterval(id);
  }, [load, live]);

  const { busy, run } = useTechLogOrders(data, () => {
    onReload();
    void load();
  });

  const systemsTab = data.source === "fenix";

  return (
    <div className="space-y-3">
      {systemsTab && (
        <div className="inline-flex rounded-lg border border-sf-border bg-sf-bg/40 p-0.5">
          {(["plan", "systems"] as const).map((v) => (
            <button
              key={v}
              onClick={() => setView(v)}
              className={cn(
                "rounded-md px-3 py-1 text-[11px] font-medium",
                view === v ? "bg-sf-raised text-sf-text" : "text-sf-muted hover:text-sf-text",
              )}
            >
              {t(`techlog.plan.view_${v}`)}
            </button>
          ))}
        </div>
      )}

      {view === "systems" && systemsTab ? (
        <>
          <p className="text-[11px] text-sf-muted">{t("techlog.plan.systems_hint", { app: data.appName })}</p>
          <CabinSection registration={data.registration} />
        </>
      ) : page === null ? (
        <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.plan.loading")} />
      ) : (
        <>
          {!live && (
            <TechLogPageNotice state={page.state} appName={data.appName} activeRegistration={page.activeRegistration} what="cabin" />
          )}
          {page.plan && page.plan.seats.length === 0 && (
            <p className="text-[11px] text-sf-muted">{t("techlog.plan.no_cabin")}</p>
          )}
          {page.plan ? (
            <PlanView
              data={data}
              plan={page.plan}
              savedAt={live ? null : page.savedAt}
              sel={sel}
              setSel={setSel}
              busy={busy}
              run={run}
            />
          ) : data.source === "simfleet" ? (
            <p className="text-[11px] text-sf-muted">{t("techlog.plan.no_cabin")}</p>
          ) : (
            !live && <p className="text-[11px] text-sf-muted">{t("techlog.plan.never_seen")}</p>
          )}
        </>
      )}
    </div>
  );
}

type RunFn = ReturnType<typeof useTechLogOrders>["run"];

function PlanView({
  data,
  plan,
  savedAt,
  sel,
  setSel,
  busy,
  run,
}: {
  data: TechLogAircraft;
  plan: TechLogCabinPlan;
  savedAt: string | null;
  sel: Selection;
  setSel: (s: Selection) => void;
  busy: string | null;
  run: RunFn;
}) {
  const tones = useMemo(() => {
    const n: Record<SeatTone, number> = { good: 0, worn: 0, low: 0, end: 0, defect: 0, blocked: 0 };
    for (const s of plan.seats) n[seatTone(s)] += 1;
    return n;
  }, [plan.seats]);
  const withDefects = plan.seats.filter((s) => s.broken.length > 0).length;
  const failed = plan.fittings.filter((f) => f.status === "Failed").length;
  const seat = sel?.kind === "seat" ? plan.seats.find((s) => s.seat === sel.seat) ?? null : null;
  // (03/11) Limpieza de vistas: el plano a la vista; el resto, en un modal.
  const [detail, setDetail] = useState<"condition" | "fittings" | null>(null);
  const tab = sel?.kind === "spot" ? "fittings" : detail;
  const condition = (
    <div className="flex h-2 w-full overflow-hidden rounded-full bg-sf-raised">
      {(Object.keys(SEAT_FILL) as SeatTone[]).map((k) =>
        tones[k] > 0 ? (
          <div key={k} style={{ width: `${(tones[k] / Math.max(1, plan.seats.length)) * 100}%`, background: SEAT_FILL[k] }} />
        ) : null,
      )}
    </div>
  );

  return (
    <div className="space-y-3">
      {plan.seats.length > 0 && (
      <Section
        icon={<Armchair className="h-4 w-4 text-brand-300" />}
        title={t("techlog.plan.title", { n: plan.seats.length })}
        subtitle={savedAt ? t("techlog.plan.saved_at", { d: fmtDate(savedAt) }) : undefined}
        action={
          <span className="text-[11px] text-sf-muted">
            {t("techlog.plan.header_counts", { d: withDefects, b: plan.blockedSeats, f: failed, n: plan.fittings.length })}
          </span>
        }
      >
        <SeatMap plan={plan} sel={sel} onSeat={(s) => setSel({ kind: "seat", seat: s })} onSpot={(type, end) => setSel({ kind: "spot", type, end })} />
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-sf-muted">
          {(Object.keys(SEAT_FILL) as SeatTone[]).map((k) => (
            <span key={k} className="inline-flex items-center gap-1">
              <span className="h-2.5 w-2.5 rounded-sm" style={{ background: SEAT_FILL[k] }} />
              {t(`techlog.plan.tone_${k}`)} {tones[k]}
            </span>
          ))}
          <span className="ml-auto">{t("techlog.plan.tap_hint")}</span>
        </div>
      </Section>
      )}

      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        {plan.seats.length > 0 && (
          <button
            onClick={() => setDetail("condition")}
            className="flex flex-col gap-2 rounded-xl border border-sf-border bg-sf-bg/40 p-3 text-left transition-colors hover:border-brand-500/40 hover:bg-sf-raised/40"
          >
            <span className="flex w-full items-center gap-2 text-xs font-semibold text-sf-text">
              <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-300" />
              <span className="min-w-0 flex-1 truncate">{t("techlog.plan.by_condition")}</span>
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sf-muted/60" />
            </span>
            {condition}
          </button>
        )}
        <button
          onClick={() => setDetail("fittings")}
          className={cn(
            "flex flex-col gap-1.5 rounded-xl border border-sf-border bg-sf-bg/40 p-3 text-left transition-colors hover:border-brand-500/40 hover:bg-sf-raised/40",
            plan.seats.length === 0 && "md:col-span-2",
          )}
        >
          <span className="flex w-full items-center gap-2 text-xs font-semibold text-sf-text">
            <Wrench className="h-4 w-4 shrink-0 text-amber-300" />
            <span className="min-w-0 flex-1 truncate">{t("techlog.plan.fittings_title")}</span>
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sf-muted/60" />
          </span>
          <span className={cn("text-[11px]", failed > 0 ? "text-rose-300" : "text-sf-muted")}>
            {t("techlog.plan.fittings_line", { n: plan.fittings.length, f: failed })}
          </span>
        </button>
      </div>

      {seat && (
        <DetailModal
          title={t("techlog.plan.seat_modal", { seat: seat.seat })}
          icon={<Armchair className="h-4 w-4" />}
          onClose={() => setSel(null)}
        >
          <SeatDetail data={data} seat={seat} busy={busy} run={run} onClose={() => setSel(null)} />
        </DetailModal>
      )}
      {tab && (
        <DetailModal
          title={t("techlog.plan.title", { n: plan.seats.length })}
          icon={<Armchair className="h-4 w-4" />}
          tabs={[
            ...(plan.seats.length > 0 ? [{ id: "condition", label: t("techlog.plan.by_condition") }] : []),
            { id: "fittings", label: t("techlog.plan.fittings_title"), badge: failed > 0 ? failed : undefined },
          ]}
          tab={tab}
          onTab={(id) => {
            setSel(null);
            setDetail(id as "condition" | "fittings");
          }}
          onClose={() => {
            setDetail(null);
            if (sel?.kind === "spot") setSel(null);
          }}
        >
          {tab === "condition" ? (
            <>
              {condition}
              <div className="mt-3 grid grid-cols-3 gap-2 text-center lg:grid-cols-6">
                {(Object.keys(SEAT_FILL) as SeatTone[]).map((k) => (
                  <div key={k} className="rounded-lg bg-sf-raised/50 px-1 py-1.5">
                    <div className="text-sm font-semibold tabular-nums text-sf-text">{tones[k]}</div>
                    <div className="text-[10px] text-sf-muted">{t(`techlog.plan.tone_${k}`)}</div>
                  </div>
                ))}
              </div>
              <p className="mt-3 text-[11px] text-sf-muted">{t("techlog.plan.seats_hint")}</p>
            </>
          ) : (
            <FittingsList data={data} fittings={plan.fittings} sel={sel} clearSel={() => setSel(null)} busy={busy} run={run} />
          )}
        </DetailModal>
      )}
    </div>
  );
}

/** Reparar / reemplazar (las órdenes de piezas de siempre, con su cola). */
function OrderButton({
  k,
  label,
  onClick,
  busy,
  disabled,
}: {
  k: string;
  label: string;
  onClick: () => void;
  busy: string | null;
  disabled?: boolean;
}) {
  return (
    <button
      disabled={disabled || busy !== null}
      onClick={onClick}
      className="inline-flex shrink-0 items-center gap-1 rounded-md border border-sf-border bg-sf-raised px-2 py-1 text-[11px] font-medium text-sf-text transition-colors hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {busy === k && <Loader2 className="h-3 w-3 animate-spin" />}
      {label}
    </button>
  );
}

function FittingsList({
  data,
  fittings,
  sel,
  clearSel,
  busy,
  run,
}: {
  data: TechLogAircraft;
  fittings: TechLogFitting[];
  sel: Selection;
  clearSel: () => void;
  busy: string | null;
  run: RunFn;
}) {
  const spotSel = sel?.kind === "spot" ? sel : null;
  const shown = spotSel
    ? fittings.filter((f) => {
        const s = spotOf(f);
        return s.type === spotSel.type && (spotSel.end == null || s.end === spotSel.end);
      })
    : fittings;
  const groups = useMemo(() => {
    const m = new Map<string, TechLogFitting[]>();
    for (const f of shown) m.set(f.system || "—", [...(m.get(f.system || "—") ?? []), f]);
    return [...m.entries()];
  }, [shown]);
  const basisUnit = (b: string) => {
    const k = `techlog.plan.basis_${b.toLowerCase()}`;
    const v = t(k);
    return v === k ? b : v;
  };
  return (
    <Section
      icon={<Wrench className="h-4 w-4 text-amber-300" />}
      title={t("techlog.plan.fittings_title")}
      action={
        spotSel ? (
          <button onClick={clearSel} className="inline-flex items-center gap-1 rounded-md bg-brand-500/15 px-2 py-0.5 text-[10px] text-brand-200">
            {t(`techlog.plan.spot_${spotSel.type}`)}
            {spotSel.end ? ` · ${spotSel.end}` : ""}
            <X className="h-3 w-3" />
          </button>
        ) : undefined
      }
    >
      {shown.length === 0 ? (
        <Empty icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />} text={t("techlog.plan.no_fittings")} />
      ) : (
        <div className="grid grid-cols-1 gap-x-6 gap-y-3 2xl:grid-cols-2">
          {groups.map(([system, list]) => (
            <div key={system}>
              <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">{system}</div>
              <ul className="divide-y divide-sf-border/40">
                {list.map((f) => {
                  const inWork = f.work != null;
                  const canRepair =
                    !inWork && f.kind.toLowerCase() !== "wear" && f.status !== "Failed" && (f.status !== "Ok" || f.worn) && (f.repairsLeft ?? 1) > 0;
                  // La traducción puede dejar fuera la posición ("AFT L"): se añade.
                  const label = f.position && !f.name.toUpperCase().includes(f.position.toUpperCase()) ? `${f.name} ${f.position}` : f.name;
                  return (
                    <li key={f.id} className="flex items-center gap-2 py-1.5">
                      <div className="min-w-0 flex-1">
                        <div className={cn("flex items-center gap-1.5 text-xs", f.status === "Failed" ? "font-semibold text-rose-300" : "text-sf-text")}>
                          <span className="truncate">{label}</span>
                          {f.status !== "Ok" && (
                            <span
                              className={cn(
                                "rounded px-1 text-[9px] font-bold uppercase",
                                f.status === "Failed" ? "bg-rose-500/20 text-rose-300" : "bg-amber-500/20 text-amber-300",
                              )}
                            >
                              {t(f.status === "Failed" ? "techlog.parts.failed" : "techlog.parts.degraded")}
                            </span>
                          )}
                          {inWork && <span className="rounded bg-brand-500/20 px-1 text-[9px] font-bold uppercase text-brand-200">{t("techlog.plan.in_work")}</span>}
                        </div>
                        <div className="mt-0.5 truncate text-[10px] tabular-nums text-sf-muted">
                          {Math.round(f.used).toLocaleString()} / {Math.round(f.life).toLocaleString()} {basisUnit(f.basis)}
                        </div>
                      </div>
                      <div className="flex w-24 shrink-0 items-center gap-1.5">
                        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sf-raised">
                          <div className={cn("h-full rounded-full", barColor(f.remainingPct))} style={{ width: `${Math.max(2, Math.min(100, f.remainingPct))}%` }} />
                        </div>
                        <span className="w-8 text-right text-[10px] tabular-nums text-sf-muted">{Math.round(f.remainingPct)}%</span>
                      </div>
                      {!inWork && (
                        <div className="flex shrink-0 gap-1">
                          {canRepair && (
                            <OrderButton
                              k={`pfix-${f.id}`}
                              label={t("techlog.repair")}
                              busy={busy}
                              onClick={() => void run(`pfix-${f.id}`, { action: "partFix", ids: [f.id], label: `${t("techlog.repair")} · ${f.name}` })}
                            />
                          )}
                          <OrderButton
                            k={`prep-${f.id}`}
                            label={t("techlog.replace")}
                            busy={busy}
                            onClick={() => void run(`prep-${f.id}`, { action: "partReplace", ids: [f.id], label: `${t("techlog.replace")} · ${f.name}` })}
                          />
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>
      )}
      <p className="mt-2 text-[10px] text-sf-muted">
        {data.source === "simfleet" ? t("techlog.plan.orders_hint_own") : t("techlog.plan.orders_hint", { app: data.appName })}
      </p>
    </Section>
  );
}

function SeatDetail({
  data,
  seat,
  busy,
  run,
  onClose,
}: {
  data: TechLogAircraft;
  seat: TechLogSeat;
  busy: string | null;
  run: RunFn;
  onClose: () => void;
}) {
  // Los defectos del asiento (lo roto) son defectos de TechLog con su posición.
  const defects = data.defects.filter((d) => (d.position ?? "").toUpperCase() === seat.seat.toUpperCase());
  const tone = seatTone(seat);
  return (
    <section className="rounded-xl border border-brand-500/40 bg-sf-bg/60 p-3">
      <header className="flex items-center gap-2">
        <span className="h-3 w-3 rounded-sm" style={{ background: SEAT_FILL[tone] }} />
        <h3 className="text-sm font-semibold text-sf-text">{t("techlog.plan.seat", { s: seat.seat })}</h3>
        <span className="text-[11px] text-sf-muted">
          {t(data.source === "simfleet" ? "techlog.plan.seat_line_own" : "techlog.plan.seat_line", {
            c: Math.round(seat.cycles).toLocaleString(),
            p: seat.worstRemainingPct != null ? Math.round(seat.worstRemainingPct) : "—",
          })}
        </span>
        <button onClick={onClose} className="ml-auto rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text">
          <X className="h-3.5 w-3.5" />
        </button>
      </header>
      {seat.items && seat.items.length > 0 ? (
        // (02/11) Cabina propia: cada pieza con su vida y su orden.
        <ul className="mt-2 divide-y divide-sf-border/40">
          {seat.items.map((it) => (
            <li key={it.id} className="flex items-center gap-2 py-1.5">
              <span className={cn("min-w-0 flex-1 truncate text-xs", it.status === "Failed" ? "font-semibold text-rose-300" : "text-sf-text")}>
                {t(`techlog.plan.item_${it.key}`)}
                {it.status === "Failed" && ` · ${t("techlog.parts.failed")}`}
              </span>
              <div className="flex w-24 shrink-0 items-center gap-1.5">
                <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sf-raised">
                  <div className={cn("h-full rounded-full", barColor(it.remainingPct))} style={{ width: `${Math.max(2, Math.min(100, it.remainingPct))}%` }} />
                </div>
                <span className="w-8 text-right text-[10px] tabular-nums text-sf-muted">{Math.round(it.remainingPct)}%</span>
              </div>
              <OrderButton
                k={`prep-${it.id}`}
                label={t("techlog.replace")}
                busy={busy}
                onClick={() =>
                  void run(`prep-${it.id}`, {
                    action: "partReplace",
                    ids: [it.id],
                    label: `${t("techlog.replace")} · ${t(`techlog.plan.item_${it.key}`)} ${seat.seat}`,
                  })
                }
              />
            </li>
          ))}
        </ul>
      ) : seat.broken.length > 0 ? (
        <ul className="mt-1.5 space-y-0.5 text-[11px] text-rose-300">
          {seat.broken.map((b) => (
            <li key={b}>• {b}</li>
          ))}
        </ul>
      ) : (
        <p className="mt-1.5 text-[11px] text-sf-muted">{t("techlog.plan.seat_ok")}</p>
      )}
      {seat.blocked && <p className="mt-1 text-[11px] text-amber-300">{t("techlog.plan.seat_blocked")}</p>}
      {defects.length > 0 && (
        <ul className="mt-2 divide-y divide-sf-border/40">
          {defects.map((d) => (
            <li key={d.id} className="flex flex-wrap items-center gap-2 py-1.5">
              <span className="rounded bg-sf-raised px-1.5 py-0.5 font-mono text-[10px] font-bold text-sf-text">{d.category === "NoGo" ? "NO-GO" : d.category}</span>
              <span className="min-w-0 flex-1 truncate text-xs text-sf-text">{d.title}</span>
              <OrderButton k={`fix-${d.id}`} label={t("techlog.fix")} busy={busy} onClick={() => void run(`fix-${d.id}`, { action: "fix", id: d.id, label: `${t("techlog.fix")} · ${d.title}` })} />
              {d.status === "Open" && d.canDefer && d.latestDeferral && (
                <OrderButton
                  k={`defer-${d.id}`}
                  label={t("techlog.defer_until", { d: fmtDate(d.latestDeferral) })}
                  busy={busy}
                  onClick={() => void run(`defer-${d.id}`, { action: "defer", id: d.id, until: d.latestDeferral!, label: `${t("techlog.defer_until", { d: fmtDate(d.latestDeferral) })} · ${d.title}` })}
                />
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** Pasillos según los asientos por fila (3-3, 2-4-2, 3-3-3, 3-4-3…). */
function aisleBreaks(n: number): number[] {
  if (n <= 4) return [Math.ceil(n / 2)];
  if (n <= 6) return [Math.ceil(n / 2)];
  if (n === 7) return [2, 5];
  if (n === 8) return [2, 6];
  if (n === 9) return [3, 6];
  return [3, n - 3];
}

/**
 * Plano propio de la cabina visto desde arriba, morro a la izquierda: fila 1
 * delante, la letra A abajo (lado izquierdo del avión). Delante y detrás, los
 * galleys (G) y baños (L) se pueden tocar para filtrar sus equipos.
 */
function SeatMap({
  plan,
  sel,
  onSeat,
  onSpot,
}: {
  plan: TechLogCabinPlan;
  sel: Selection;
  onSeat: (seat: string) => void;
  onSpot: (type: Spot["type"], end: Spot["end"]) => void;
}) {
  const letters = (plan.letters || "ABCDEF").split("");
  const rows = Math.max(plan.rows, ...plan.seats.map((s) => s.row), 1);
  const bySeat = new Map(plan.seats.map((s) => [`${s.row}${s.letter}`, s]));
  const breaks = aisleBreaks(letters.length);
  const CW = 15;
  const CH = 13;
  const AISLE = 11;
  const X0 = 74;
  const Y0 = 22;
  // Letra A abajo: el índice crece hacia arriba.
  const yOf = (li: number) => {
    const fromTop = letters.length - 1 - li;
    const aislesAbove = breaks.filter((b) => letters.length - b <= fromTop).length;
    return Y0 + fromTop * CH + aislesAbove * AISLE;
  };
  const cabinH = letters.length * CH + breaks.length * AISLE;
  const W = X0 + rows * CW + 70;
  const H = Y0 * 2 + cabinH;
  const midY = Y0 + cabinH / 2;
  const fittingsAt = (type: Spot["type"], end: Spot["end"]) =>
    plan.fittings.filter((f) => {
      const s = spotOf(f);
      return s.type === type && s.end === end;
    });
  const spotTone = (type: Spot["type"], end: Spot["end"]) => {
    const fs = fittingsAt(type, end);
    if (fs.length === 0) return null;
    if (fs.some((f) => f.status === "Failed")) return "#f43f5e";
    const worst = Math.min(...fs.map((f) => f.remainingPct));
    return worst < 10 ? "#f87171" : worst < 30 ? "#fbbf24" : worst < 60 ? "#a3e635" : "#34d399";
  };
  const Spot = ({ type, end, x, y, w, h, label }: { type: Spot["type"]; end: Spot["end"]; x: number; y: number; w: number; h: number; label: string }) => {
    const tone = spotTone(type, end);
    const on = sel?.kind === "spot" && sel.type === type && sel.end === end;
    return (
      <g role="button" tabIndex={0} className="cursor-pointer" onClick={() => onSpot(type, end)} onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onSpot(type, end)}>
        <rect x={x} y={y} width={w} height={h} rx={3} style={{ fill: "rgb(var(--sf-raised))", stroke: on ? "#35cdb2" : tone ?? "rgb(var(--sf-muted) / 0.4)" }} strokeWidth={on ? 2 : 1.2} />
        <text x={x + w / 2} y={y + h / 2 + 3.5} textAnchor="middle" fontSize="9" fontWeight={700} style={{ fill: tone ?? "rgb(var(--sf-muted))" }}>
          {label}
        </text>
      </g>
    );
  };
  const exitRow = Math.max(2, Math.round(rows * 0.42));
  const line = "rgb(var(--sf-muted) / 0.45)";
  return (
    <div className="overflow-x-auto">
      <svg viewBox={`0 0 ${W} ${H}`} className="block h-auto min-w-[560px] w-full" aria-label={t("techlog.plan.map")}>
        {/* Fuselaje: morro a la izquierda, cola a la derecha */}
        <path
          d={`M ${X0 - 34} ${Y0 - 10} L ${W - 52} ${Y0 - 10} Q ${W - 8} ${midY - cabinH / 4} ${W - 4} ${midY} Q ${W - 8} ${midY + cabinH / 4} ${W - 52} ${H - Y0 + 10} L ${X0 - 34} ${H - Y0 + 10} Q 6 ${H - Y0 + 4} 4 ${midY} Q 6 ${Y0 - 4} ${X0 - 34} ${Y0 - 10} Z`}
          style={{ fill: "rgb(var(--sf-bg) / 0.5)", stroke: line }}
          strokeWidth="1.3"
        />
        {/* Puertas (1L/1R delante, 2L/2R detrás) */}
        {[
          [X0 - 30, Y0 - 13],
          [X0 - 30, H - Y0 + 7],
          [W - 76, Y0 - 13],
          [W - 76, H - Y0 + 7],
        ].map(([x, y]) => (
          <rect key={`${x}-${y}`} x={x} y={y} width={16} height={6} rx={1.5} style={{ fill: "#38bdf8", opacity: 0.55 }} />
        ))}
        {/* Salidas sobre las alas */}
        {[exitRow, exitRow + 1].map((r) => (
          <g key={r}>
            <line x1={X0 + (r - 1) * CW + 2} x2={X0 + r * CW - 3} y1={Y0 - 9} y2={Y0 - 9} stroke="#34d399" strokeWidth="2" />
            <line x1={X0 + (r - 1) * CW + 2} x2={X0 + r * CW - 3} y1={H - Y0 + 9} y2={H - Y0 + 9} stroke="#34d399" strokeWidth="2" />
          </g>
        ))}
        {/* Galleys y baños delante y detrás */}
        <Spot type="galley" end="FWD" x={X0 - 30} y={midY - 14} w={20} h={28} label="G" />
        <Spot type="lav" end="FWD" x={X0 - 30} y={Y0 - 2} w={20} h={18} label="L" />
        <Spot type="crew" end="FWD" x={X0 - 30} y={H - Y0 - 16} w={20} h={14} label="FA" />
        <Spot type="galley" end="AFT" x={W - 64} y={midY - 14} w={22} h={28} label="G" />
        <Spot type="lav" end="AFT" x={W - 64} y={Y0 - 2} w={22} h={18} label="L" />
        <Spot type="crew" end="AFT" x={W - 64} y={H - Y0 - 16} w={22} h={14} label="FA" />
        {/* Asientos */}
        {Array.from({ length: rows }, (_, i) => i + 1).map((row) => (
          <g key={row}>
            {letters.map((letter, li) => {
              const s = bySeat.get(`${row}${letter}`);
              if (!s) return null;
              const x = X0 + (row - 1) * CW;
              const y = yOf(li);
              const tone = seatTone(s);
              const on = sel?.kind === "seat" && sel.seat === s.seat;
              return (
                <g key={letter} role="button" tabIndex={0} className="cursor-pointer" onClick={() => onSeat(s.seat)} onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onSeat(s.seat)}>
                  <title>{`${s.seat} · ${s.worstRemainingPct != null ? Math.round(s.worstRemainingPct) : "—"}%`}</title>
                  <rect
                    x={x + 1}
                    y={y + 1}
                    width={CW - 3}
                    height={CH - 3}
                    rx={2.5}
                    style={{ fill: SEAT_FILL[tone], opacity: tone === "blocked" ? 0.9 : 0.85, stroke: on ? "#ffffff" : "none" }}
                    strokeWidth={on ? 1.6 : 0}
                  />
                  {(tone === "defect" || tone === "blocked") && (
                    <text x={x + CW / 2 - 0.5} y={y + CH / 2 + 2.5} textAnchor="middle" fontSize="8" fontWeight={800} fill="#ffffff">
                      {tone === "blocked" ? "×" : "!"}
                    </text>
                  )}
                </g>
              );
            })}
            {(row === 1 || row % 5 === 0 || row === rows) && (
              <text x={X0 + (row - 1) * CW + (CW - 3) / 2 + 1} y={H - 4} textAnchor="middle" fontSize="7.5" style={{ fill: "rgb(var(--sf-muted))" }}>
                {row}
              </text>
            )}
          </g>
        ))}
        {/* Letras de cada fila */}
        {letters.map((l, li) => (
          <text key={l} x={X0 - 6} y={yOf(li) + CH / 2 + 2} textAnchor="end" fontSize="7.5" style={{ fill: "rgb(var(--sf-muted))" }}>
            {l}
          </text>
        ))}
      </svg>
    </div>
  );
}
