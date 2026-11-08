import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Legend,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { AlertTriangle, CheckCircle2, FileJson, FileText, Loader2, PlaneLanding, Trash2 } from "lucide-react";
import type {
  TechLogAircraft,
  TechLogFlightDetail,
  TechLogFlightRow,
  TechLogFlightsPage,
  TechLogLanding,
  TechLogTakeoff,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useTechLogStore } from "../stores/useTechLogStore";
import { useToastStore } from "../stores/useToastStore";
import { actionError, Empty } from "./TechLogPanel";
import { DetailModal } from "./DetailModal";

/**
 * (01/11) Etapa 5a — página «Flights» de Fenix/PMDG TechLog en SimFleet:
 * cifras, tomas de contacto y lista de vuelos del avión; el detalle tiene las
 * pestañas de TechLog (informe, resumen, cronología, despegue y aterrizaje,
 * sistemas, desgaste y perfil). Se lee de la copia de su base, así que
 * funciona con TechLog cerrado; borrar un vuelo sí lo pide abierto.
 */

const DETAIL_TABS = ["report", "summary", "timeline", "takeoff", "systems", "wear", "profile"] as const;
type DetailTab = (typeof DETAIL_TABS)[number];

/** Series del perfil: validadas (daltonismo y contraste) en claro y grafito. */
const SERIES = { ias: "#22a893", gs: "#8b5cf6" };
/** Estados de una toma (con etiqueta siempre: nunca sólo el color). */
const TD = { soft: "#34d399", firm: "#fbbf24", hard: "#fb7185" };
const HARD_FPM = 600;
const FIRM_FPM = 300;

function tdKind(fpm: number): keyof typeof TD {
  const a = Math.abs(fpm);
  return a >= HARD_FPM ? "hard" : a >= FIRM_FPM ? "firm" : "soft";
}

function fmtBlock(min: number | null | undefined): string {
  if (min == null || !Number.isFinite(min)) return "—";
  const m = Math.round(min);
  return `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}`;
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

function fmtUtc(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "—" : `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}Z`;
}

const num = (v: number | null | undefined, digits = 0) =>
  v == null || !Number.isFinite(v) ? "—" : v.toLocaleString(undefined, { maximumFractionDigits: digits, minimumFractionDigits: digits });

/** (02/11) Fase del FlightBook ("initial_climb") en el idioma de la app. */
function phaseName(p: string): string {
  const k = `fb.checklist.phase.${p}`;
  const v = t(k);
  return v === k ? p : v;
}

/** (02/11) Regla del FlightBook en la que se perdieron puntos. */
function ruleName(r: { ruleId: string; label: string }): string {
  const k = `fb.rule.${r.ruleId}`;
  const v = t(k);
  return v === k ? r.label : v;
}

/** (02/11) Registro de texto de un vuelo del Tech Log propio. */
function ownTextLog(d: TechLogFlightDetail, reg: string): string {
  const f = d.flight;
  const L: string[] = [
    `SimFleet Tech Log · ${t("techlog.flights.log_title")}`,
    `${reg}   ${fmtWhen(f.outUtc ?? f.openedUtc)}   ${route(f)}   #${f.id}`,
    `${t("techlog.flights.block")} ${fmtBlock(f.blockMin)} · ${t("techlog.flights.distance")} ${num(f.distanceNm)} nm · ${t("techlog.flights.pax")} ${num(f.pax)} · ${t("techlog.flights.score")} ${f.score ?? "—"}`,
  ];
  if (d.fuelBurnKg != null) L.push(`${t("techlog.flights.fuel_burned")}: ${num(d.fuelBurnKg)} kg`);
  const l = d.landings[0];
  if (l) L.push(`${t("techlog.flights.touchdown")}: ${num(l.vsFpm)} fpm · ${num(l.peakG, 2)} g · ${l.airport ?? ""}`);
  if (d.summary) L.push("", d.summary);
  if (d.scoreItems?.length) {
    L.push("", t("techlog.flights.log_score"));
    for (const s of d.scoreItems) L.push(`  −${num(s.lost)} ${ruleName(s)}`);
  }
  L.push("", t("techlog.flights.tab_timeline"));
  for (const e of d.events) {
    L.push(`  ${fmtUtc(e.utc).padEnd(6)}  ${e.kind === "PHASE" ? phaseName(e.text) : `${e.severity.padEnd(8)} ${e.text}`}`);
  }
  if (d.effects.length) {
    L.push("", t("techlog.flights.tab_systems"));
    for (const x of d.effects) L.push(`  ${x.title}${x.outcome ? ` (${x.outcome})` : ""}`);
  }
  if (d.wear.length) {
    L.push("", t("techlog.flights.tab_wear"));
    for (const w of d.wear) L.push(`  ${w.item}: ${w.amount > 0 ? "+" : "−"}${num(Math.abs(w.amount), 2)} ${w.unit}`);
  }
  return L.join("\r\n");
}

function route(f: TechLogFlightRow): string {
  return `${f.from ?? "?"} → ${f.to ?? "?"}`;
}

/** Tinta de los gráficos según el tema (rejilla y ejes discretos). */
export function useChartInk() {
  const read = () => {
    const cs = getComputedStyle(document.documentElement);
    const rgb = (name: string, a = 1) => {
      const [r, g, b] = cs.getPropertyValue(name).trim().split(/\s+/).map(Number);
      return Number.isFinite(r) ? `rgba(${r},${g},${b},${a})` : `rgba(161,161,166,${a})`;
    };
    return { grid: rgb("--sf-muted", 0.16), axis: rgb("--sf-muted"), surface: rgb("--sf-surface"), text: rgb("--sf-text") };
  };
  const [ink, setInk] = useState(read);
  useEffect(() => {
    const mo = new MutationObserver(() => setInk(read()));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => mo.disconnect();
  }, []);
  return ink;
}

export function TechLogFlights({
  data,
  refreshKey,
  focus,
}: {
  data: TechLogAircraft;
  refreshKey: number;
  /** Vuelo pedido desde el FlightBook. */
  focus: { id: number; n: number } | null;
}) {
  const epoch = useTechLogStore((s) => s.epoch);
  const [page, setPage] = useState<TechLogFlightsPage | null | undefined>(undefined);
  const [selected, setSelected] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const p = await api.techlogFlights(data.registration, 100);
      setPage(p);
      setSelected((cur) => (cur != null && p?.flights.some((f) => f.id === cur) ? cur : null));
    } catch {
      setPage(null);
    }
  }, [data.registration]);
  useEffect(() => {
    void load();
  }, [load, epoch, refreshKey]);
  useEffect(() => {
    if (focus) setSelected(focus.id);
  }, [focus]);

  if (page === undefined) return <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.flights.loading")} />;
  if (page === null || page.flights.length === 0) {
    return <Empty icon={<PlaneLanding className="h-4 w-4" />} text={t("techlog.flights.none", { app: data.appName })} />;
  }
  // (03/11) Limpieza de vistas: lista y cifras a la vista; el detalle de un
  // vuelo, con todas sus pestañas, en un modal.
  const open = page.flights.find((f) => f.id === selected) ?? null;
  return (
    <div className="grid grid-cols-1 items-start gap-3 lg:grid-cols-2">
      <FlightsOverview flights={page.flights} />
      <FlightList flights={page.flights} selected={selected} onSelect={setSelected} />
      {open && (
        <DetailModal
          title={`#${open.id} · ${route(open)}`}
          icon={<PlaneLanding className="h-4 w-4" />}
          onClose={() => setSelected(null)}
        >
          <FlightDetail
            key={open.id}
            data={data}
            flightId={open.id}
            onDeleted={() => {
              setSelected(null);
              void load();
            }}
          />
        </DetailModal>
      )}
    </div>
  );
}

function FlightsOverview({ flights }: { flights: TechLogFlightRow[] }) {
  const ink = useChartInk();
  const fpms = flights.map((f) => f.landing?.vsFpm).filter((v): v is number => v != null);
  const scores = flights.map((f) => f.score).filter((v): v is number => v != null);
  const block = flights.reduce((a, f) => a + (f.blockMin ?? 0), 0);
  const dist = flights.reduce((a, f) => a + (f.distanceNm ?? 0), 0);
  const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
  const softest = fpms.length ? fpms.reduce((a, b) => (Math.abs(b) < Math.abs(a) ? b : a)) : null;
  const chart = [...flights]
    .reverse()
    .filter((f) => f.landing?.vsFpm != null)
    .map((f) => ({ id: f.id, label: `#${f.id}`, route: route(f), when: fmtWhen(f.inUtc ?? f.openedUtc), fpm: Math.abs(f.landing!.vsFpm!), raw: f.landing!.vsFpm! }));
  const stats: [string, string][] = [
    [t("techlog.flights.stat_flights"), String(flights.length)],
    [t("techlog.flights.stat_block"), fmtBlock(block)],
    [t("techlog.flights.stat_distance"), `${num(dist)} nm`],
    [t("techlog.flights.stat_avg_td"), fpms.length ? `${num(avg(fpms))} fpm` : "—"],
    [t("techlog.flights.stat_softest"), softest != null ? `${num(softest)} fpm` : "—"],
    [t("techlog.flights.stat_avg_score"), scores.length ? num(avg(scores)) : "—"],
  ];
  return (
    <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-3">
      <div className="grid grid-cols-3 gap-1.5">
        {stats.map(([k, v]) => (
          <div key={k} className="rounded-lg bg-sf-raised/50 px-2 py-1.5">
            <div className="text-[9.5px] uppercase tracking-wide text-sf-muted">{k}</div>
            <div className="text-sm font-semibold tabular-nums text-sf-text">{v}</div>
          </div>
        ))}
      </div>
      {chart.length > 0 && (
        <>
          <div className="mt-3 flex items-baseline justify-between">
            <h4 className="text-[11px] font-semibold text-sf-text">{t("techlog.flights.td_title")}</h4>
            <span className="text-[10px] text-sf-muted">{t("techlog.flights.td_hint")}</span>
          </div>
          <div className="h-32 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={chart} margin={{ top: 8, right: 4, left: -8, bottom: 0 }} barCategoryGap={2}>
                <CartesianGrid stroke={ink.grid} vertical={false} />
                <XAxis dataKey="label" hide />
                <YAxis tick={{ fill: ink.axis, fontSize: 10 }} axisLine={false} tickLine={false} width={40} domain={[0, (max: number) => Math.max(HARD_FPM + 100, max)]} />
                <ReferenceLine
                  y={HARD_FPM}
                  stroke={TD.hard}
                  strokeDasharray="4 3"
                  label={{ value: t("techlog.flights.td_hard_line"), position: "insideTopRight", fill: ink.axis, fontSize: 9 }}
                />
                <Tooltip cursor={{ fill: ink.grid }} content={<TdTooltip />} />
                <Bar dataKey="fpm" radius={[4, 4, 0, 0]} maxBarSize={28} isAnimationActive={false}>
                  {chart.map((d) => (
                    <Cell key={d.id} fill={TD[tdKind(d.fpm)]} />
                  ))}
                </Bar>
              </BarChart>
            </ResponsiveContainer>
          </div>
        </>
      )}
    </section>
  );
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function TdTooltip({ active, payload }: any) {
  if (!active || !payload?.length) return null;
  const p = payload[0].payload as { label: string; route: string; when: string; raw: number; fpm: number };
  const k = tdKind(p.fpm);
  return (
    <div className="rounded-lg border border-sf-border bg-sf-surface/95 px-3 py-2 text-xs shadow-xl">
      <div className="font-semibold text-sf-text">
        {p.label} · {p.route}
      </div>
      <div className="text-sf-muted">{p.when}</div>
      <div className="mt-1 flex items-center gap-1.5 text-sf-text">
        <span className="h-2 w-2 rounded-sm" style={{ background: TD[k] }} />
        {num(p.raw)} fpm · {t(`techlog.flights.td_${k}`)}
      </div>
    </div>
  );
}

function ScoreRing({ score, size = 30 }: { score: number | null; size?: number }) {
  const r = size / 2 - 3;
  const c = 2 * Math.PI * r;
  const pct = score == null ? 0 : Math.max(0, Math.min(100, score)) / 100;
  const color = score == null ? "#7a7a80" : score >= 85 ? "#34d399" : score >= 65 ? "#fbbf24" : "#fb7185";
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="shrink-0">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" style={{ stroke: "rgb(var(--sf-raised))" }} strokeWidth={3} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={color}
        strokeWidth={3}
        strokeLinecap="round"
        strokeDasharray={`${c * pct} ${c}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      <text x={size / 2} y={size / 2 + 3.5} textAnchor="middle" fontSize={size > 40 ? 13 : 9.5} fontWeight={700} style={{ fill: "rgb(var(--sf-text))" }}>
        {score == null ? "—" : Math.round(score)}
      </text>
    </svg>
  );
}

function FlightList({
  flights,
  selected,
  onSelect,
}: {
  flights: TechLogFlightRow[];
  selected: number | null;
  onSelect: (id: number) => void;
}) {
  return (
    <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-1.5">
      <div className="px-2 pb-1 pt-1 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">
        {t("techlog.flights.list_title", { n: flights.length })}
      </div>
      <ul className="max-h-72 space-y-1 overflow-y-auto pr-0.5">
        {flights.map((f) => {
          const fpm = f.landing?.vsFpm;
          return (
            <li key={f.id}>
              <button
                onClick={() => onSelect(f.id)}
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors",
                  selected === f.id ? "border-brand-500/50 bg-brand-500/10" : "border-transparent hover:bg-sf-raised/60",
                )}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-xs font-semibold text-sf-text">
                    <span className="text-sf-muted">#{f.id}</span>
                    {route(f)}
                    {f.status !== "closed" && (
                      <span className="rounded bg-sky-500/20 px-1 text-[9px] font-bold uppercase text-sky-300">{t("techlog.flights.open")}</span>
                    )}
                  </div>
                  <div className="mt-0.5 text-[10px] text-sf-muted">
                    {fmtWhen(f.outUtc ?? f.openedUtc)} · {fmtBlock(f.blockMin)} · {num(f.distanceNm)} nm
                    {fpm != null && (
                      <span className="ml-1" style={{ color: TD[tdKind(fpm)] }}>
                        {" "}
                        · {num(fpm)} fpm
                      </span>
                    )}
                  </div>
                </div>
                {f.warnings > 0 && (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded bg-amber-500/15 px-1 py-0.5 text-[10px] font-semibold text-amber-300" title={t("techlog.flights.warnings_hint")}>
                    <AlertTriangle className="h-3 w-3" />
                    {f.warnings}
                  </span>
                )}
                <ScoreRing score={f.score} />
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function FlightDetail({ data, flightId, onDeleted }: { data: TechLogAircraft; flightId: number; onDeleted: () => void }) {
  const pushToast = useToastStore((s) => s.push);
  const epoch = useTechLogStore((s) => s.epoch);
  const [d, setD] = useState<TechLogFlightDetail | null | undefined>(undefined);
  const [tab, setTab] = useState<DetailTab>("report");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  useEffect(() => {
    let alive = true;
    api
      .techlogFlightDetail(data.registration, flightId)
      .then((r) => alive && setD(r))
      .catch(() => alive && setD(null));
    return () => {
      alive = false;
    };
  }, [data.registration, flightId, epoch]);

  // (02/11) Vuelos del Tech Log propio: se borran desde el FlightBook.
  const own = data.source === "simfleet";
  const save = async (kind: "log" | "json") => {
    const ext = kind === "log" ? "txt" : "json";
    const path = await api
      .pickSavePath(`techlog-${data.registration}-${flightId}.${ext}`, [{ name: kind === "log" ? t("techlog.flights.file_text") : "JSON", extensions: [ext] }])
      .catch(() => null);
    if (!path) return;
    setBusy(kind);
    try {
      if (own && d) {
        // (02/11) Tech Log propio: lo escribe SimFleet desde lo que ya tiene.
        const content = kind === "log" ? ownTextLog(d, data.registration) : JSON.stringify(d, null, 2);
        await api.saveBinaryFile(path, btoa(unescape(encodeURIComponent(content))));
      } else {
        await api.techlogFlightSave(data.registration, flightId, kind, path);
      }
      pushToast({ kind: "success", title: t("techlog.flights.saved"), message: path });
    } catch (e) {
      const msg = String(e);
      pushToast({ kind: "error", title: t("techlog.flights.save_failed"), message: msg.includes("not_found") ? t("techlog.flights.no_log") : msg });
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    setBusy("delete");
    try {
      const r = await api.techlogFlightDelete(data.registration, flightId);
      pushToast({
        kind: r.ok ? "success" : "error",
        title: r.ok ? t("techlog.flights.deleted", { id: flightId }) : t("techlog.flights.delete_refused"),
        message: r.message || undefined,
      });
      if (r.ok) onDeleted();
    } catch (e) {
      const msg = String(e);
      pushToast({
        kind: "error",
        title: t("techlog.flights.delete_refused"),
        message: msg.includes("closed") ? t("techlog.flights.delete_needs_app", { app: data.appName }) : actionError(msg),
      });
    } finally {
      setBusy(null);
      setConfirmDelete(false);
    }
  };

  if (d === undefined) return <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.flights.loading")} />;
  if (d === null) return <Empty icon={<PlaneLanding className="h-4 w-4" />} text={t("techlog.flights.not_found")} />;
  const f = d.flight;
  const btn =
    "inline-flex items-center gap-1 rounded-md border border-sf-border bg-sf-raised px-2 py-1 text-[11px] font-medium text-sf-text hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-40";
  return (
    <section className="min-w-0 rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-semibold text-sf-text">
          <span className="text-sf-muted">#{f.id}</span> {route(f)}
        </h3>
        <span className="text-[11px] text-sf-muted">{fmtWhen(f.outUtc ?? f.openedUtc)}</span>
        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          <button className={btn} disabled={busy !== null || (!d.hasLog && !data.running)} onClick={() => void save("log")}>
            {busy === "log" ? <Loader2 className="h-3 w-3 animate-spin" /> : <FileText className="h-3 w-3" />}
            {t("techlog.flights.text_log")}
          </button>
          <button className={btn} disabled={busy !== null} onClick={() => void save("json")}>
            {busy === "json" ? <Loader2 className="h-3 w-3 animate-spin" /> : <FileJson className="h-3 w-3" />}
            JSON
          </button>
          {own ? null : confirmDelete ? (
            <>
              <button
                className="inline-flex items-center gap-1 rounded-md border border-rose-500/50 bg-rose-500/15 px-2 py-1 text-[11px] font-medium text-rose-200 hover:bg-rose-500/25 disabled:opacity-40"
                disabled={busy !== null}
                onClick={() => void remove()}
              >
                {busy === "delete" ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
                {t("techlog.flights.delete_confirm", { id: f.id })}
              </button>
              <button className={btn} onClick={() => setConfirmDelete(false)}>
                {t("common.cancel")}
              </button>
            </>
          ) : (
            <button className={btn} disabled={busy !== null} onClick={() => setConfirmDelete(true)} title={t("techlog.flights.delete_hint", { app: data.appName })}>
              <Trash2 className="h-3 w-3" />
              {t("techlog.flights.delete")}
            </button>
          )}
        </div>
      </header>
      {confirmDelete && <p className="mt-1.5 text-[11px] text-rose-300">{t("techlog.flights.delete_warning", { app: data.appName })}</p>}

      <div className="mt-3 flex flex-wrap gap-1 border-b border-sf-border pb-2">
        {DETAIL_TABS.map((k) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            className={cn(
              "rounded-md px-2.5 py-1 text-[11px] font-medium",
              tab === k ? "bg-brand-500/15 text-brand-200 ring-1 ring-brand-500/40" : "text-sf-muted hover:bg-sf-raised hover:text-sf-text",
            )}
          >
            {t(`techlog.flights.tab_${k}`)}
          </button>
        ))}
      </div>
      <HideEmptyRows.Provider value={own}>
      <div className="mt-3">
        {tab === "report" && <ReportTab d={d} />}
        {tab === "summary" && <SummaryTab d={d} />}
        {tab === "timeline" && <TimelineTab d={d} />}
        {tab === "takeoff" && <TakeoffTab d={d} />}
        {tab === "systems" && <SystemsTab d={d} />}
        {tab === "wear" && <WearTab d={d} />}
        {tab === "profile" && <ProfileTab d={d} />}
      </div>
      </HideEmptyRows.Provider>
    </section>
  );
}

function Card({ title, children, className }: { title: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("rounded-lg border border-sf-border bg-sf-surface p-3", className)}>
      <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">{title}</div>
      {children}
    </div>
  );
}

/** (02/11) Vuelos del Tech Log propio: SimFleet no tiene todos los datos de
 *  TechLog; las filas sin dato no se enseñan. */
const HideEmptyRows = createContext(false);

function Row({ k, v, tone }: { k: string; v: React.ReactNode; tone?: string }) {
  const hide = useContext(HideEmptyRows);
  if (hide && typeof v === "string" && v.replace(/—|kg|kt|fpm|IAS|GS|°|%|\/|·|\s|\bm\b/g, "") === "") return null;
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-xs">
      <span className="text-sf-muted">{k}</span>
      <span className={cn("text-right tabular-nums", tone ?? "text-sf-text")}>{v}</span>
    </div>
  );
}

function ReportTab({ d }: { d: TechLogFlightDetail }) {
  const r = d.report;
  const f = d.flight;
  const burn = d.fuelOutKg != null && d.fuelInKg != null ? d.fuelOutKg - d.fuelInKg : null;
  const fuel = d.fuelBurnKg ?? burn;
  const used = [
    ...(fuel != null ? [{ item: t("techlog.flights.fuel_burned"), amount: fuel, unit: "kg" }] : []),
    ...d.wear.filter((w) => w.unit !== "%").slice(0, 6),
  ];
  const clean = r ? r.after.failed <= r.before.failed && d.effects.length === 0 : d.effects.length === 0;
  const raised = d.effects;
  if (!r) {
    return <p className="text-[11px] text-sf-muted">{t(d.source === "simfleet" ? "techlog.flights.no_report_own" : "techlog.flights.no_report")}</p>;
  }
  const groups = [...r.groups].sort((a, b) => b.usedPct - a.usedPct).slice(0, 10);
  const maxUsed = Math.max(0.0001, ...groups.map((g) => g.usedPct));
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-4">
        <HealthRing value={r.healthBefore} label={t("techlog.flights.before")} />
        <span className="text-sf-muted">→</span>
        <HealthRing value={r.healthAfter} label={t("techlog.flights.after")} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-sm font-semibold text-sf-text">
            {clean ? <CheckCircle2 className="h-4 w-4 text-emerald-400" /> : <AlertTriangle className="h-4 w-4 text-amber-300" />}
            {clean ? t("techlog.flights.clean") : t("techlog.flights.some_wear")}
          </div>
          <p className="text-[11px] text-sf-muted">
            {t("techlog.flights.health_line", {
              n: r.before.good + r.before.low + r.before.failed,
              a: num(r.healthBefore, 1),
              b: num(r.healthAfter, 1),
              d: num(r.healthAfter - r.healthBefore, 2),
            })}
          </p>
        </div>
      </div>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
        <Card title={t("techlog.flights.parts_by_stage")}>
          <Row k={t("techlog.flights.stage_good")} v={`${r.before.good} → ${r.after.good}`} />
          <Row k={t("techlog.flights.stage_low")} v={`${r.before.low} → ${r.after.low}`} tone={r.after.low > r.before.low ? "text-amber-300" : undefined} />
          <Row k={t("techlog.flights.stage_failed")} v={`${r.before.failed} → ${r.after.failed}`} tone={r.after.failed > r.before.failed ? "text-rose-300" : undefined} />
        </Card>
        <Card title={t("techlog.flights.used_up")}>
          {used.length === 0 ? (
            <p className="text-[11px] text-sf-muted">—</p>
          ) : (
            used.map((w) => <Row key={w.item} k={w.item} v={`${num(Math.abs(w.amount), Math.abs(w.amount) < 10 ? 1 : 0)} ${w.unit}`} />)
          )}
        </Card>
        <Card title={t("techlog.flights.flight")}>
          <Row k={t("techlog.flights.block")} v={fmtBlock(f.blockMin)} />
          <Row k={t("techlog.flights.distance")} v={`${num(f.distanceNm)} nm`} />
          <Row k={t("techlog.flights.score")} v={f.score != null ? num(f.score) : "—"} />
        </Card>
      </div>
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <Card title={t("techlog.flights.life_by_system", { n: r.groups.reduce((a, g) => a + g.parts, 0) })}>
          <ul className="space-y-1.5">
            {groups.map((g) => (
              <li key={g.name}>
                <div className="flex items-baseline justify-between gap-2 text-xs">
                  <span className="text-sf-text">
                    {g.name} <span className="text-[10px] text-sf-muted">· {t("techlog.flights.n_parts", { n: g.parts })}</span>
                  </span>
                  <span className="tabular-nums text-sf-muted">{num(g.usedPct, 3)}%</span>
                </div>
                <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-sf-raised">
                  <div className="h-full rounded-full bg-amber-400/80" style={{ width: `${Math.max(1.5, (g.usedPct / maxUsed) * 100)}%` }} />
                </div>
              </li>
            ))}
          </ul>
        </Card>
        <div className="space-y-3">
          <Card title={t("techlog.flights.aged_most")}>
            {r.agedMost.length === 0 ? (
              <p className="text-[11px] text-sf-muted">—</p>
            ) : (
              r.agedMost.map((a) => (
                <Row key={a.item} k={a.item} v={`${num(a.leftPct)}% · −${num(a.usedPct, 2)}`} tone={a.leftPct < 30 ? "text-amber-300" : undefined} />
              ))
            )}
          </Card>
          <Card title={t("techlog.flights.broke_raised")}>
            {r.crossed.length === 0 && raised.length === 0 ? (
              <p className="text-[11px] text-sf-muted">{t("techlog.flights.nothing_broke")}</p>
            ) : (
              <ul className="space-y-1">
                {raised.map((x, i) => (
                  <li key={`e${i}`} className="rounded-md bg-rose-500/10 px-2 py-1 text-[11px] text-rose-200">
                    {x.title}
                    {x.outcome && <span className="text-rose-300/80"> · {x.outcome}</span>}
                  </li>
                ))}
                {r.crossed.map((c) => (
                  <li key={c.item} className="rounded-md bg-amber-500/10 px-2 py-1 text-[11px] text-amber-200">
                    {t(c.stage === "failed" ? "techlog.flights.crossed_failed" : "techlog.flights.crossed_low", { item: c.item })}
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}

function HealthRing({ value, label }: { value: number; label: string }) {
  const size = 64;
  const r = size / 2 - 5;
  const c = 2 * Math.PI * r;
  const pct = Math.max(0, Math.min(100, value)) / 100;
  const color = value >= 60 ? "#34d399" : value >= 30 ? "#fbbf24" : "#fb7185";
  return (
    <div className="flex flex-col items-center gap-0.5">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" style={{ stroke: "rgb(var(--sf-raised))" }} strokeWidth={5} />
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke={color} strokeWidth={5} strokeLinecap="round" strokeDasharray={`${c * pct} ${c}`} transform={`rotate(-90 ${size / 2} ${size / 2})`} />
        <text x={size / 2} y={size / 2 + 4.5} textAnchor="middle" fontSize={13} fontWeight={700} style={{ fill: "rgb(var(--sf-text))" }}>
          {Math.round(value)}
        </text>
      </svg>
      <span className="text-[10px] text-sf-muted">{label}</span>
    </div>
  );
}

function SummaryTab({ d }: { d: TechLogFlightDetail }) {
  const f = d.flight;
  const burn = d.fuelOutKg != null && d.fuelInKg != null ? d.fuelOutKg - d.fuelInKg : null;
  const tow = d.takeoff?.mtowKg;
  const mlw = d.landings.find((l) => !l.touchAndGo)?.mlwKg ?? d.landings[0]?.mlwKg;
  return (
    <div className="space-y-3">
      {d.summary && <p className="text-xs leading-relaxed text-sf-text">{d.summary}</p>}
      {d.scoreNotes.length > 0 && (
        <ul className="space-y-0.5 text-[11px] text-amber-200">
          {d.scoreNotes.map((n) => (
            <li key={n}>• {n}</li>
          ))}
        </ul>
      )}
      {(d.scoreItems?.length ?? 0) > 0 && (
        <ul className="space-y-0.5 text-[11px] text-amber-200">
          {d.scoreItems!.map((s) => (
            <li key={s.ruleId}>
              • −{num(s.lost)} {ruleName(s)}
            </li>
          ))}
        </ul>
      )}
      <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
        <Card title={t("techlog.flights.times")}>
          <Row k="OUT" v={fmtUtc(f.outUtc)} />
          <Row k="OFF" v={fmtUtc(d.offUtc)} />
          <Row k="ON" v={fmtUtc(d.onUtc)} />
          <Row k="IN" v={fmtUtc(f.inUtc)} />
          <Row k={t("techlog.flights.block")} v={fmtBlock(f.blockMin)} />
          <Row k={t("techlog.flights.air")} v={fmtBlock(f.airMin)} />
        </Card>
        <Card title={t("techlog.flights.fuel")}>
          {d.fuelBurnKg == null && (
            <>
              <Row k="OUT" v={`${num(d.fuelOutKg)} kg`} />
              <Row k="OFF" v={`${num(d.fuelOffKg)} kg`} />
              <Row k="ON" v={`${num(d.fuelOnKg)} kg`} />
              <Row k="IN" v={`${num(d.fuelInKg)} kg`} />
            </>
          )}
          <Row k={t("techlog.flights.burn")} v={(d.fuelBurnKg ?? burn) != null ? `${num(d.fuelBurnKg ?? burn)} kg` : "—"} />
        </Card>
        <Card title={t("techlog.flights.weights")}>
          <Row k="ZFW" v={`${num(d.zfwKg)} kg`} />
          <Row k="TOW" v={tow ? `${num(d.towKg)} / ${num(tow)} kg` : `${num(d.towKg)} kg`} tone={tow && d.towKg && d.towKg > tow ? "text-rose-300" : undefined} />
          <Row k="LW" v={mlw ? `${num(d.lwKg)} / ${num(mlw)} kg` : `${num(d.lwKg)} kg`} tone={mlw && d.lwKg && d.lwKg > mlw ? "text-rose-300" : undefined} />
          <Row k={t("techlog.flights.pax")} v={num(f.pax)} />
          <Row k={t("techlog.flights.max_alt")} v={f.maxAltFt != null ? `FL${Math.round(f.maxAltFt / 100)}` : "—"} />
        </Card>
      </div>
    </div>
  );
}

function TimelineTab({ d }: { d: TechLogFlightDetail }) {
  const [all, setAll] = useState(false);
  const events = all
    ? d.events
    : d.events.filter((e) => e.severity !== "Info" || e.kind === "PHASE" || /TAKEOFF|LANDING|TOUCH|OUT|OFF|ON|IN\b/i.test(e.kind));
  const tone = (s: string) => (s === "Warning" ? "text-rose-300" : s === "Caution" ? "text-amber-300" : "text-sf-text");
  const mmss = (s: number | null) => {
    if (s == null) return "";
    const a = Math.abs(Math.round(s));
    return `T${s < 0 ? "−" : "+"}${Math.floor(a / 60)}:${String(a % 60).padStart(2, "0")}`;
  };
  return (
    <div>
      <label className="mb-2 inline-flex cursor-pointer items-center gap-1.5 text-[11px] text-sf-muted">
        <input type="checkbox" className="accent-brand-500" checked={all} onChange={(e) => setAll(e.target.checked)} />
        {t("techlog.flights.show_all_events", { n: d.events.length })}
      </label>
      <ul className="divide-y divide-sf-border/40">
        {events.map((e) => (
          <li key={e.id} className="flex items-baseline gap-3 py-1.5 text-xs">
            <span className="w-14 shrink-0 font-mono text-[10px] text-sf-muted">{fmtUtc(e.utc)}</span>
            <span className="w-16 shrink-0 font-mono text-[10px] text-sf-muted">{mmss(e.t)}</span>
            <span className="w-24 shrink-0 truncate text-[10px] uppercase text-sf-muted">
              {e.kind === "PHASE" ? "" : e.phase ? phaseName(e.phase) : ""}
            </span>
            <span className={cn("min-w-0 flex-1", e.kind === "PHASE" ? "font-semibold text-brand-200" : tone(e.severity))}>
              {e.kind === "PHASE" ? phaseName(e.text) : e.text}
              {e.value != null && e.limit != null && <span className="text-sf-muted"> ({num(e.value)} / {num(e.limit)})</span>}
            </span>
            {e.altFt != null && <span className="shrink-0 tabular-nums text-[10px] text-sf-muted">{num(e.altFt)} ft</span>}
          </li>
        ))}
      </ul>
    </div>
  );
}

function TakeoffTab({ d }: { d: TechLogFlightDetail }) {
  const to: TechLogTakeoff | null = d.takeoff;
  const brakes = (xs?: number[] | null) => (xs && xs.length ? xs.map((x) => `${Math.round(x)}°`).join(" · ") : "—");
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <Card title={t("techlog.flights.takeoff")}>
        {!to ? (
          <p className="text-[11px] text-sf-muted">—</p>
        ) : (
          <>
            <Row k={t("techlog.flights.runway")} v={[to.airport, to.runway].filter(Boolean).join(" ") || "—"} />
            <Row k={t("techlog.flights.weight")} v={to.mtowKg ? `${num(to.weightKg)} / ${num(to.mtowKg)} kg` : `${num(to.weightKg)} kg`} />
            <Row k={t("techlog.flights.flaps_thrust")} v={`${num(to.flapsHandle)} · ${to.thrust ?? "—"}${to.flexTempC != null ? ` ${num(to.flexTempC)}°` : ""}`} />
            <Row k={t("techlog.flights.rotate_liftoff")} v={`${num(to.rotateIas)} / ${num(to.liftoffIas)} kt`} />
            <Row k={t("techlog.flights.pitch_ground")} v={`${num(to.maxPitchOnGround, 1)}° (${t("techlog.flights.tail_at", { p: num(to.tailstrikePitch, 1) })})`} />
            <Row k={t("techlog.flights.max_n1_egt")} v={`${num(to.maxN1, 1)}%${to.maxEgtC != null ? ` · ${num(to.maxEgtC)} °C` : ""}`} />
            <Row k={t("techlog.flights.roll")} v={`${num(to.rollDistanceM)} m`} />
            <Row k={t("techlog.flights.wind")} v={`${num(to.windHeadKt)} / ${num(to.windCrossKt)} kt`} />
            <Row k={t("techlog.flights.brakes_release")} v={brakes(to.brakeTempAtReleaseC)} />
          </>
        )}
      </Card>
      {d.landings.length === 0 ? (
        <Card title={t("techlog.flights.landing")}>
          <p className="text-[11px] text-sf-muted">—</p>
        </Card>
      ) : (
        d.landings.map((l, i) => <LandingCard key={i} l={l} n={d.landings.length > 1 ? i + 1 : null} brakes={brakes} />)
      )}
    </div>
  );
}

function LandingCard({ l, n, brakes }: { l: TechLogLanding; n: number | null; brakes: (xs?: number[] | null) => string }) {
  const k = l.vsFpm != null ? tdKind(l.vsFpm) : null;
  const gate = (g?: TechLogLanding["gate1000"]) =>
    g ? `${num(g.ias)} kt · ${num(g.vsFpm)} fpm · ${g.gearDown ? t("techlog.flights.gear_down") : t("techlog.flights.gear_up")} · F${num(g.flapsHandle)}` : "—";
  return (
    <Card title={`${l.touchAndGo ? t("techlog.flights.touch_and_go") : t("techlog.flights.landing")}${n ? ` ${n}` : ""}`}>
      <Row k={t("techlog.flights.runway")} v={[l.airport, l.runway].filter(Boolean).join(" ") || "—"} />
      <Row
        k={t("techlog.flights.touchdown")}
        v={
          <span className="inline-flex items-center gap-1.5">
            {k && <span className="h-2 w-2 rounded-sm" style={{ background: TD[k] }} />}
            {num(l.vsFpm)} fpm · {num(l.peakG, 2)} g{k ? ` · ${t(`techlog.flights.td_${k}`)}` : ""}
          </span>
        }
      />
      <Row k={t("techlog.flights.speeds")} v={`${num(l.ias)} IAS · ${num(l.gs)} GS`} />
      <Row k={t("techlog.flights.attitude")} v={`${num(l.pitch, 1)}° / ${num(l.bank, 1)}° · ${t("techlog.flights.crab", { c: num(l.crabDeg, 1) })}`} />
      <Row
        k={t("techlog.flights.wind")}
        v={`${l.windDir == null ? "—" : num(((l.windDir % 360) + 360) % 360)}°/${num(l.windKt)} kt · ${num(l.headwindKt)} / ${num(l.crosswindKt)} kt`}
      />
      <Row k={t("techlog.flights.threshold")} v={`${num(l.distancePastThresholdM)} m · ${num(l.centrelineOffsetM, 1)} m`} />
      <Row k={t("techlog.flights.flare_bounces")} v={`${num(l.flareSec, 1)} s · ${num(l.bounces)}`} />
      <Row k={t("techlog.flights.gate1000")} v={gate(l.gate1000)} />
      <Row k={t("techlog.flights.gate500")} v={gate(l.gate500)} />
      <Row k={t("techlog.flights.stable")} v={l.stable == null ? "—" : l.stable ? t("techlog.flights.yes") : t("techlog.flights.no")} tone={l.stable === false ? "text-amber-300" : undefined} />
      <Row k={t("techlog.flights.rollout")} v={`${num(l.rolloutDistanceM)} m · ${t("techlog.flights.reverse", { p: num(l.maxReversePct) })}`} />
      <Row k={t("techlog.flights.brakes_peak")} v={brakes(l.peakBrakeTempC)} />
    </Card>
  );
}

function SystemsTab({ d }: { d: TechLogFlightDetail }) {
  if (d.effects.length === 0) {
    return <Empty icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />} text={t("techlog.flights.no_effects")} />;
  }
  return (
    <ul className="divide-y divide-sf-border/40">
      {d.effects.map((x, i) => (
        <li key={i} className="py-2">
          <div className="flex flex-wrap items-center gap-1.5 text-xs">
            {x.ata != null && <span className="rounded bg-sf-raised px-1 font-mono text-[10px] text-sf-muted">ATA {x.ata}</span>}
            <span className="text-[10px] uppercase text-sf-muted">{x.system}</span>
            <span className="font-medium text-sf-text">{x.title}</span>
            {x.origin && <span className="text-[10px] text-sf-muted">· {x.origin}</span>}
          </div>
          {x.restriction && <div className="mt-0.5 text-[11px] text-amber-200">{x.restriction}</div>}
          {(x.outcome || x.complianceNote) && (
            <div className="mt-0.5 text-[11px] text-sf-muted">
              {x.complied != null && (x.complied ? `${t("techlog.flights.complied")} · ` : `${t("techlog.flights.not_complied")} · `)}
              {[x.outcome, x.complianceNote].filter(Boolean).join(" · ")}
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}

function WearTab({ d }: { d: TechLogFlightDetail }) {
  const rows = [...d.wear].sort((a, b) => Math.abs(b.amount) - Math.abs(a.amount));
  if (rows.length === 0) return <Empty icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />} text={t("techlog.flights.no_wear")} />;
  return (
    <table className="w-full text-xs">
      <thead>
        <tr className="text-left text-[10px] uppercase tracking-wide text-sf-muted">
          <th className="py-1 pr-2 font-medium">{t("techlog.flights.col_item")}</th>
          <th className="py-1 pr-2 text-right font-medium">{t("techlog.flights.col_amount")}</th>
          <th className="py-1 font-medium">{t("techlog.flights.col_detail")}</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-sf-border/40">
        {rows.map((w, i) => (
          <tr key={i}>
            <td className="py-1 pr-2 text-sf-text">{w.item}</td>
            <td className="py-1 pr-2 text-right tabular-nums text-sf-text">
              {w.amount > 0 ? "+" : "−"}
              {num(Math.abs(w.amount), Math.abs(w.amount) < 10 ? 2 : 0)} {w.unit}
            </td>
            <td className="py-1 text-[11px] text-sf-muted">{w.detail ?? ""}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Perfil del vuelo: tres gráficos con el mismo eje de tiempo (nunca dos escalas en uno). */
function ProfileTab({ d }: { d: TechLogFlightDetail }) {
  const ink = useChartInk();
  const rows = useMemo(() => {
    const tr = d.track;
    if (!tr) return [];
    const at = (name: string) => tr.columns.indexOf(name);
    const [it, ialt, iias, igs, ivs] = [at("t"), at("alt"), at("ias"), at("gs"), at("vs")];
    return tr.rows
      .filter((r) => r[it] != null)
      .map((r) => ({
        min: Math.round(((r[it] as number) / 60) * 10) / 10,
        alt: ialt >= 0 ? r[ialt] : null,
        ias: iias >= 0 ? r[iias] : null,
        gs: igs >= 0 ? r[igs] : null,
        vs: ivs >= 0 ? r[ivs] : null,
      }));
  }, [d.track]);
  if (rows.length < 2) {
    return <Empty icon={<PlaneLanding className="h-4 w-4" />} text={t(d.source === "simfleet" ? "techlog.flights.no_track_own" : "techlog.flights.no_track")} />;
  }
  const axis = { tick: { fill: ink.axis, fontSize: 10 }, axisLine: false, tickLine: false } as const;
  const xAxis = (show: boolean) => (
    <XAxis dataKey="min" type="number" domain={["dataMin", "dataMax"]} tickFormatter={(v: number) => `${Math.round(v)}′`} hide={!show} {...axis} />
  );
  const sync = `tl-profile-${d.flight.id}`;
  return (
    <div className="space-y-2">
      <ChartBlock title={t("techlog.flights.profile_alt")}>
        <AreaChart data={rows} syncId={sync} margin={{ top: 6, right: 10, left: 0, bottom: 0 }}>
          <defs>
            <linearGradient id={`${sync}-alt`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={SERIES.ias} stopOpacity={0.3} />
              <stop offset="100%" stopColor={SERIES.ias} stopOpacity={0} />
            </linearGradient>
          </defs>
          <CartesianGrid stroke={ink.grid} vertical={false} />
          {xAxis(false)}
          <YAxis {...axis} width={52} tickFormatter={(v: number) => `${Math.round(v / 1000)}k`} />
          <Tooltip content={<ProfileTooltip />} cursor={{ stroke: ink.axis, strokeWidth: 1 }} />
          <Area type="linear" dataKey="alt" name={t("techlog.flights.series_alt")} stroke={SERIES.ias} strokeWidth={2} fill={`url(#${sync}-alt)`} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
        </AreaChart>
      </ChartBlock>
      <ChartBlock title={t("techlog.flights.profile_speed")}>
        <LineChart data={rows} syncId={sync} margin={{ top: 6, right: 10, left: 0, bottom: 0 }}>
          <CartesianGrid stroke={ink.grid} vertical={false} />
          {xAxis(false)}
          <YAxis {...axis} width={52} />
          <Tooltip content={NoBox} cursor={{ stroke: ink.axis, strokeWidth: 1 }} />
          <Legend verticalAlign="top" align="right" height={18} iconType="plainline" wrapperStyle={{ fontSize: 10, color: ink.axis }} />
          <Line type="linear" dataKey="ias" name="IAS" stroke={SERIES.ias} strokeWidth={2} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
          <Line type="linear" dataKey="gs" name="GS" stroke={SERIES.gs} strokeWidth={2} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
        </LineChart>
      </ChartBlock>
      <ChartBlock title={t("techlog.flights.profile_vs")}>
        <LineChart data={rows} syncId={sync} margin={{ top: 6, right: 10, left: 0, bottom: 0 }}>
          <CartesianGrid stroke={ink.grid} vertical={false} />
          {xAxis(true)}
          <YAxis {...axis} width={52} />
          <ReferenceLine y={0} stroke={ink.axis} strokeOpacity={0.5} />
          <Tooltip content={NoBox} cursor={{ stroke: ink.axis, strokeWidth: 1 }} />
          <Line type="linear" dataKey="vs" name={t("techlog.flights.series_vs")} stroke={SERIES.ias} strokeWidth={2} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
        </LineChart>
      </ChartBlock>
    </div>
  );
}

function ChartBlock({ title, children }: { title: string; children: React.ReactElement }) {
  return (
    <div className="rounded-lg border border-sf-border bg-sf-surface px-2 pb-1 pt-2">
      <div className="px-1 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">{title}</div>
      <div className="h-36 w-full">
        <ResponsiveContainer width="100%" height="100%">
          {children}
        </ResponsiveContainer>
      </div>
    </div>
  );
}

/** Los gráficos sincronizados sólo dibujan la línea: un único recuadro arriba. */
const NoBox = () => null;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function ProfileTooltip({ active, payload }: any) {
  if (!active || !payload?.length) return null;
  const p = payload[0].payload as { min: number; alt: number | null; ias: number | null; gs: number | null; vs: number | null };
  return (
    <div className="rounded-lg border border-sf-border bg-sf-surface/95 px-3 py-2 text-xs shadow-xl">
      <div className="font-semibold text-sf-text">T+{Math.floor(p.min)}′</div>
      <Row k={t("techlog.flights.series_alt")} v={`${num(p.alt)} ft`} />
      <Row k="IAS" v={`${num(p.ias)} kt`} />
      <Row k="GS" v={`${num(p.gs)} kt`} />
      <Row k={t("techlog.flights.series_vs")} v={`${num(p.vs)} fpm`} />
    </div>
  );
}
