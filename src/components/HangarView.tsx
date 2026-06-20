import { useEffect, useMemo, useState } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  ResponsiveContainer,
  LabelList,
} from "recharts";
import {
  Plane,
  Route,
  PlaneLanding,
  Gauge,
  Search,
  SlidersHorizontal,
  Download,
  ChevronDown,
  MoreVertical,
  Info,
  Film,
  ChevronRight,
  AlertTriangle,
  Play,
  Star,
  Trash2,
  MapPin,
  Building2,
  Globe2,
  LayoutGrid,
} from "lucide-react";
import type {
  HangarAnalytics,
  HangarAircraft,
  HangarLanding,
  HangarCount,
  LandingClip,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { AirlineLogo } from "./AirlineLogo";
import { cleanAtcType } from "../lib/aircraft";
import { airportRegion } from "../lib/oaciRegion";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v6 #2a) Vista "Hangar & Fleet Analytics" — rediseño maestro/detalle.
 *
 * Cabecera: título + KPIs globales (millas, aterrizajes, FPM medio) + perfil
 * del piloto con nivel/XP. Izquierda: la flota (top 10) con buscador y
 * selección. Derecha: detalle del avión seleccionado — banner, pestañas,
 * salud del tren (derivada del FPM) y tendencia FPM de los últimos 10
 * aterrizajes (recharts), más la tira "Best Landings".
 */

const GRADE_COLOR: Record<string, string> = {
  butter: "#3fbf78",
  acceptable: "#f59e0b",
  hard: "#f43f5e",
};

export function HangarView() {
  const [data, setData] = useState<HangarAnalytics | null>(null);
  const [clips, setClips] = useState<LandingClip[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const reloadClips = () => {
    api
      .listLandingClips()
      .then(setClips)
      .catch(() => {});
  };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([
      api.hangarAnalytics(),
      api.listLandingClips().catch(() => [] as LandingClip[]),
    ])
      .then(([d, c]) => {
        if (cancelled) return;
        setData(d);
        setClips(c);
        // (v6.1 #29) NO auto-seleccionamos: por defecto se muestra el
        // resumen de flota (destinos/aerolíneas/tipos/regiones).
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // (v6.1 #33) Top 10 por defecto; al buscar, filtra TODA la flota.
  const fleet = useMemo(() => {
    if (!data) return [];
    const q = search.trim().toLowerCase();
    if (!q) return data.aircraft.slice(0, 10);
    return data.aircraft.filter((a) =>
      [a.registration, a.model, a.airlineName]
        .filter(Boolean)
        .some((s) => s!.toLowerCase().includes(q)),
    );
  }, [data, search]);

  // (v6.1 #29) null = vista de resumen de flota (sin avión seleccionado).
  const selected = data?.aircraft.find((a) => a.key === selectedKey) ?? null;

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-slate-500">
        {t("common.loading")}
      </div>
    );
  }
  if (error) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-rose-400">
        {error}
      </div>
    );
  }
  if (!data || data.totalFlights === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 text-slate-500">
        <Plane className="h-8 w-8" />
        <p className="text-sm">{t("hangar.empty")}</p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto pb-8">
      {/* ── Cabecera ───────────────────────────────────────────── */}
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-slate-100">
            {t("hangar.heading")}
          </h1>
          <p className="text-xs text-slate-500">{t("hangar.subheading")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <HeaderKpi
            icon={<Route className="h-4 w-4" />}
            label={t("hangar.kpi.miles")}
            value={`${Math.round(data.totalNm).toLocaleString()} nm`}
          />
          <HeaderKpi
            icon={<PlaneLanding className="h-4 w-4" />}
            label={t("hangar.kpi.landings")}
            value={data.totalLandings.toLocaleString()}
          />
          <HeaderKpi
            icon={<Gauge className="h-4 w-4" />}
            label={t("hangar.kpi.global_fpm")}
            value={
              data.globalAvgFpm != null
                ? `${Math.round(data.globalAvgFpm)} fpm`
                : "—"
            }
          />
        </div>
      </div>

      {/* ── Cuerpo maestro/detalle ─────────────────────────────── */}
      <div className="mt-5 grid grid-cols-1 gap-4 lg:grid-cols-[320px_1fr]">
        {/* Flota */}
        <section className="rounded-2xl border border-slate-800 bg-slate-900/40 p-3">
          <h2 className="mb-2 flex items-center gap-1.5 px-1 text-sm font-semibold text-slate-200">
            {t("hangar.fleet")}
            <Info className="h-3.5 w-3.5 text-slate-600" />
          </h2>
          <div className="mb-3 flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("hangar.search")}
                className="w-full rounded-lg border border-slate-800 bg-slate-950/60 py-1.5 pl-8 pr-2 text-xs text-slate-200 placeholder:text-slate-600 focus:border-brand-500/50 focus:outline-none"
              />
            </div>
            <button className="rounded-lg border border-slate-800 bg-slate-950/60 p-1.5 text-slate-400 hover:text-slate-200">
              <SlidersHorizontal className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="space-y-1.5">
            {/* (v6.1 #29) Resumen de flota — vuelve a la vista sin selección. */}
            <button
              onClick={() => setSelectedKey(null)}
              className={`flex w-full items-center gap-2.5 rounded-xl border p-2 text-left transition-colors ${
                selected == null
                  ? "border-brand-500/60 bg-brand-500/10 ring-1 ring-brand-500/30"
                  : "border-transparent bg-slate-950/40 hover:bg-slate-800/40"
              }`}
            >
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-brand-500/15 text-brand-300">
                <LayoutGrid className="h-4 w-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-xs font-semibold text-slate-100">
                  {t("hangar.overview.title")}
                </div>
                <div className="truncate text-[11px] text-slate-500">
                  {t("hangar.overview.subtitle")}
                </div>
              </div>
            </button>
            {fleet.map((ac, i) => (
              <FleetRow
                key={ac.key}
                rank={search.trim() ? 0 : i + 1}
                ac={ac}
                active={ac.key === selected?.key}
                onClick={() => setSelectedKey(ac.key)}
              />
            ))}
            {fleet.length === 0 && (
              <p className="px-1 py-3 text-center text-[11px] text-slate-600">
                {t("hangar.search_empty")}
              </p>
            )}
          </div>
        </section>

        {/* Detalle del avión, o resumen de flota si no hay selección. */}
        {selected ? (
          <AircraftDetail
            ac={selected}
            synthetic={data.bestLandings}
            recorded={clips}
            onReloadClips={reloadClips}
          />
        ) : (
          <FleetOverview data={data} />
        )}
      </div>
    </div>
  );
}

function HeaderKpi({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center gap-2.5 rounded-xl border border-slate-800 bg-slate-900/60 px-3 py-2">
      <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15 text-brand-300">
        {icon}
      </div>
      <div>
        <div className="text-[10px] uppercase tracking-wide text-slate-500">
          {label}
        </div>
        <div className="text-sm font-semibold text-slate-100">{value}</div>
        <div className="text-[9px] text-slate-600">{t("hangar.all_aircraft")}</div>
      </div>
    </div>
  );
}

// ── (v6.1 #29) Resumen de flota — vista por defecto sin avión seleccionado ──

function FleetOverview({ data }: { data: HangarAnalytics }) {
  // Regiones más voladas: derivadas de TODOS los vuelos (store) por destino,
  // agrupando el ICAO por su zona OACI (mismo criterio que los badges).
  const entries = useFlightLogStore((s) => s.entries);
  const topRegions = useMemo<HangarCount[]>(() => {
    const counts = new Map<string, number>();
    for (const e of entries) {
      const icao = e.destinationIcao ?? null;
      if (!icao) continue;
      const label = airportRegion(icao).label;
      counts.set(label, (counts.get(label) ?? 0) + 1);
    }
    return [...counts.entries()]
      .map(([label, count]) => ({ label, code: null, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 8);
  }, [entries]);

  return (
    <section className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4">
      <h2 className="mb-1 text-lg font-bold text-slate-100">
        {t("hangar.overview.heading")}
      </h2>
      <p className="mb-4 text-xs text-slate-500">
        {t("hangar.overview.hint")}
      </p>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <OverviewCard
          icon={<MapPin className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.destinations")}
          rows={data.topDestinations.map((d) => ({
            label: d.name ?? d.icao,
            sub: d.icao,
            count: d.visits,
          }))}
        />
        <OverviewCard
          icon={<Building2 className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.airlines")}
          rows={data.topAirlines.map((a) => ({
            label: a.label,
            count: a.count,
            airlineIcao: a.code,
          }))}
        />
        <OverviewCard
          icon={<Plane className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.aircraft")}
          rows={data.topAircraftTypes.map((a) => ({
            label: cleanAtcType(a.label) ?? a.label,
            count: a.count,
          }))}
        />
        <OverviewCard
          icon={<Globe2 className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.regions")}
          rows={topRegions.map((r) => ({ label: r.label, count: r.count }))}
        />
      </div>
    </section>
  );
}

interface OverviewRow {
  label: string;
  sub?: string | null;
  count: number;
  airlineIcao?: string | null;
}

function OverviewCard({
  icon,
  title,
  rows,
}: {
  icon: React.ReactNode;
  title: string;
  rows: OverviewRow[];
}) {
  const max = rows.reduce((m, r) => Math.max(m, r.count), 0) || 1;
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-4">
      <h3 className="mb-3 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
        {icon}
        {title}
      </h3>
      {rows.length === 0 ? (
        <div className="py-6 text-center text-xs text-slate-600">
          {t("hangar.overview.no_data")}
        </div>
      ) : (
        <div className="space-y-2">
          {rows.map((r, i) => (
            <div key={i} className="flex items-center gap-2">
              {r.airlineIcao !== undefined ? (
                <AirlineLogo icao={r.airlineIcao} name={r.label} size={18} />
              ) : (
                <span className="w-4 shrink-0 text-center text-[11px] font-semibold text-slate-600">
                  {i + 1}
                </span>
              )}
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="truncate text-xs text-slate-200">
                    {r.label}
                    {r.sub && (
                      <span className="ml-1 font-mono text-[10px] text-slate-500">
                        {r.sub}
                      </span>
                    )}
                  </span>
                  <span className="shrink-0 text-xs font-semibold text-slate-300">
                    {r.count}
                  </span>
                </div>
                <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-800">
                  <div
                    className="h-full rounded-full bg-brand-500/70"
                    style={{ width: `${Math.round((r.count / max) * 100)}%` }}
                  />
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function FleetRow({
  rank,
  ac,
  active,
  onClick,
}: {
  rank: number;
  ac: HangarAircraft;
  active: boolean;
  onClick: () => void;
}) {
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : null;
  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-2.5 rounded-xl border p-2 text-left transition-colors ${
        active
          ? "border-brand-500/60 bg-brand-500/10 ring-1 ring-brand-500/30"
          : "border-transparent bg-slate-950/40 hover:bg-slate-800/40"
      }`}
    >
      <span className="w-4 shrink-0 text-center text-xs font-semibold text-slate-500">
        {rank > 0 ? rank : ""}
      </span>
      <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={32} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-xs font-semibold text-slate-100">
          {ac.airlineName ?? model ?? t("hangar.unknown_model")}
        </div>
        <div className="truncate text-[11px] text-slate-500">{model ?? "—"}</div>
        {ac.registration && (
          <div className="truncate font-mono text-[10px] text-slate-600">
            {ac.registration}
          </div>
        )}
      </div>
      <div className="shrink-0 text-right">
        <div className="text-xs font-semibold text-slate-200">
          {Math.round(ac.totalNm).toLocaleString()} nm
        </div>
        <div className="text-[10px] text-slate-500">
          {ac.flights} {t("hangar.flights_short")}
        </div>
      </div>
      <MoreVertical className="h-3.5 w-3.5 shrink-0 text-slate-600" />
    </button>
  );
}

const TABS = [
  "overview",
  "performance",
  "maintenance",
  "flights",
  "history",
  "documents",
] as const;

function AircraftDetail({
  ac,
  synthetic,
  recorded,
  onReloadClips,
}: {
  ac: HangarAircraft;
  synthetic: HangarLanding[];
  recorded: LandingClip[];
  onReloadClips: () => void;
}) {
  const [tab, setTab] = useState<(typeof TABS)[number]>("overview");
  const push = useToastStore((s) => s.push);
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : null;
  const location =
    ac.lastAirportName && ac.lastIcao
      ? `${ac.lastAirportName} (${ac.lastIcao})`
      : ac.lastIcao ?? null;

  return (
    <section className="overflow-hidden rounded-2xl border border-slate-800 bg-slate-900/40">
      {/* Banner */}
      <div className="relative overflow-hidden">
        <div className="absolute right-3 top-3 z-10 flex">
          <button
            onClick={() =>
              push({ kind: "info", title: t("hangar.export_soon"), ttlMs: 3000 })
            }
            className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-950/70 px-3 py-1.5 text-xs text-slate-200 backdrop-blur hover:border-brand-500/40"
          >
            <Download className="h-3.5 w-3.5" />
            {t("hangar.export")}
            <ChevronDown className="h-3 w-3" />
          </button>
        </div>
        <div className="flex items-end gap-4 bg-gradient-to-br from-slate-800 via-slate-900 to-brand-900/40 p-5">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="text-3xl font-bold tracking-tight text-slate-50">
                {ac.registration ?? t("hangar.unknown_model")}
              </h2>
              <span className="inline-flex items-center gap-1 rounded-full bg-brand-500/20 px-2 py-0.5 text-[10px] font-medium text-brand-300 ring-1 ring-brand-500/30">
                <span className="h-1.5 w-1.5 rounded-full bg-brand-400" />
                {t("hangar.active")}
              </span>
            </div>
            <div className="mt-1.5 flex items-center gap-2">
              <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={20} />
              <span className="text-sm font-medium text-slate-200">
                {ac.airlineName ?? "—"}
              </span>
            </div>
            <p className="mt-0.5 text-xs text-slate-400">
              {[model, location].filter(Boolean).join(" · ") || "—"}
            </p>
          </div>
          <PlaneLanding className="h-16 w-16 shrink-0 text-slate-700/60" />
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b border-slate-800 px-4">
        {TABS.map((tb) => (
          <button
            key={tb}
            onClick={() => setTab(tb)}
            className={`relative px-3 py-2.5 text-xs font-medium transition-colors ${
              tab === tb
                ? "text-brand-300"
                : "text-slate-500 hover:text-slate-300"
            }`}
          >
            {t(`hangar.tab.${tb}`)}
            {tab === tb && (
              <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-brand-400" />
            )}
          </button>
        ))}
      </div>

      <div className="p-4">
        {tab === "overview" ? (
          <>
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              <GearHealthCard ac={ac} />
              <FpmTrendCard ac={ac} />
            </div>
            <BestLandings
              recorded={recorded}
              synthetic={synthetic}
              onReload={onReloadClips}
            />
          </>
        ) : (
          <div className="flex h-40 items-center justify-center text-sm text-slate-500">
            {t("hangar.tab_soon")}
          </div>
        )}
      </div>
    </section>
  );
}

function GearHealthCard({ ac }: { ac: HangarAircraft }) {
  const pct = ac.healthPct;
  const color =
    pct >= 80 ? "text-brand-400" : pct >= 50 ? "text-amber-400" : "text-rose-400";
  const barColor =
    pct >= 80 ? "bg-brand-500" : pct >= 50 ? "bg-amber-500" : "bg-rose-500";
  const condition =
    pct >= 80
      ? t("hangar.gear.good")
      : pct >= 50
        ? t("hangar.gear.monitor")
        : t("hangar.gear.service");
  // "Próximo mantenimiento" derivado de la salud (sintético, sin tracking real).
  const nextDue = Math.max(0, Math.round(pct / 10) - 3);

  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-4">
      <h3 className="mb-3 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
        <Gauge className="h-4 w-4 text-brand-400" />
        {t("hangar.gear.title")}
        <Info className="h-3 w-3 text-slate-600" />
      </h3>
      <div className="flex items-end gap-2">
        <span className={`text-3xl font-bold ${color}`}>{pct}%</span>
        <span className="mb-1 text-xs text-slate-400">{condition}</span>
      </div>
      <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-800">
        <div className={`h-full ${barColor}`} style={{ width: `${pct}%` }} />
      </div>
      {nextDue <= 5 && (
        <div className="mt-3 flex items-center gap-1.5 text-[11px] text-amber-300">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          {t("hangar.gear.recommend", { n: String(nextDue) })}
        </div>
      )}
      <div className="mt-4 flex items-center justify-center">
        <PlaneLanding className="h-12 w-12 text-slate-700" />
      </div>
      <div className="mt-3 flex items-center justify-between border-t border-slate-800 pt-3 text-[11px]">
        <div>
          <div className="text-slate-600">{t("hangar.gear.last_inspection")}</div>
          <div className="text-slate-300">{fmtDate(ac.lastFlightAt)}</div>
        </div>
        <div className="text-right">
          <div className="text-slate-600">{t("hangar.gear.next_due")}</div>
          <div className="text-slate-300">
            {t("hangar.gear.in_landings", { n: String(nextDue) })}
          </div>
        </div>
      </div>
    </div>
  );
}

function FpmTrendCard({ ac }: { ac: HangarAircraft }) {
  const landings = ac.recentLandings;
  const n = landings.length;
  const chartData = landings.map((l, i) => ({
    label: i === n - 1 ? t("hangar.fpm.now") : String(n - i),
    fpm: l.fpm,
    grade: l.grade,
  }));
  const counts = landings.reduce(
    (acc, l) => {
      acc[l.grade] = (acc[l.grade] ?? 0) + 1;
      return acc;
    },
    {} as Record<string, number>,
  );
  const avg =
    ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)} fpm` : "—";

  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-4">
      <h3 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
        <Gauge className="h-4 w-4 text-brand-400" />
        {t("hangar.fpm.title", { n: String(n) })}
        <Info className="h-3 w-3 text-slate-600" />
      </h3>
      {n === 0 ? (
        <div className="flex h-40 items-center justify-center text-xs text-slate-600">
          {t("hangar.fpm.no_data")}
        </div>
      ) : (
        <div className="h-44 w-full">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart
              data={chartData}
              margin={{ top: 16, right: 12, left: -8, bottom: 0 }}
            >
              <CartesianGrid stroke="#1e293b" vertical={false} />
              <XAxis
                dataKey="label"
                tick={{ fill: "#64748b", fontSize: 10 }}
                axisLine={{ stroke: "#1e293b" }}
                tickLine={false}
              />
              <YAxis
                tick={{ fill: "#64748b", fontSize: 10 }}
                axisLine={false}
                tickLine={false}
                width={40}
                domain={["dataMin - 50", 50]}
              />
              <Line
                type="monotone"
                dataKey="fpm"
                stroke="#38bdf8"
                strokeWidth={2}
                dot={<FpmDot />}
                isAnimationActive={false}
              >
                <LabelList
                  dataKey="fpm"
                  position="top"
                  fontSize={9}
                  fill="#94a3b8"
                />
              </Line>
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
      {/* Leyenda */}
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-slate-800 pt-3">
        <div className="flex flex-wrap gap-3 text-[10px]">
          <Legend color={GRADE_COLOR.butter} label={t("hangar.fpm.butter")} n={counts.butter ?? 0} />
          <Legend color={GRADE_COLOR.acceptable} label={t("hangar.fpm.acceptable")} n={counts.acceptable ?? 0} />
          <Legend color={GRADE_COLOR.hard} label={t("hangar.fpm.hard")} n={counts.hard ?? 0} />
        </div>
        <div className="text-right">
          <div className="text-[9px] uppercase tracking-wide text-slate-600">
            {t("hangar.fpm.avg")}
          </div>
          <div className="text-sm font-semibold text-slate-200">{avg}</div>
        </div>
      </div>
    </div>
  );
}

function FpmDot(props: {
  cx?: number;
  cy?: number;
  payload?: { grade: string };
}) {
  const { cx, cy, payload } = props;
  if (cx == null || cy == null) return <></>;
  const color = GRADE_COLOR[payload?.grade ?? ""] ?? "#38bdf8";
  return (
    <circle cx={cx} cy={cy} r={4} fill={color} stroke="#0f172a" strokeWidth={1.5} />
  );
}

function Legend({ color, label, n }: { color: string; label: string; n: number }) {
  return (
    <span className="inline-flex items-center gap-1 text-slate-400">
      <span className="h-2 w-2 rounded-full" style={{ backgroundColor: color }} />
      {label} <span className="font-semibold text-slate-200">{n}</span>
    </span>
  );
}

function BestLandings({
  recorded,
  synthetic,
  onReload,
}: {
  recorded: LandingClip[];
  synthetic: HangarLanding[];
  onReload: () => void;
}) {
  const push = useToastStore((s) => s.push);
  const hasClips = recorded.length > 0;

  return (
    <div className="mt-5">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="flex items-center gap-1.5 text-sm font-semibold text-slate-200">
          <Film className="h-4 w-4 text-brand-400" />
          {t("hangar.best_landings.title")}
          <Info className="h-3 w-3 text-slate-600" />
        </h3>
        {hasClips && (
          <button
            onClick={() =>
              push({
                kind: "info",
                title: t("hangar.best_landings.soon"),
                ttlMs: 3000,
              })
            }
            className="inline-flex items-center gap-1 text-xs text-brand-300 hover:text-brand-200"
          >
            {t("hangar.best_landings.view_all")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {hasClips ? (
        <div className="flex gap-3 overflow-x-auto pb-2">
          {recorded.map((c) => (
            <ClipCard key={c.id} clip={c} onReload={onReload} />
          ))}
        </div>
      ) : synthetic.length > 0 ? (
        <>
          <div className="flex gap-3 overflow-x-auto pb-2">
            {synthetic.map((l, i) => (
              <LandingCard key={i} l={l} />
            ))}
          </div>
          <p className="mt-1 text-[11px] text-slate-600">
            {t("hangar.best_landings.enable_hint")}
          </p>
        </>
      ) : (
        <div className="flex flex-col items-center justify-center gap-1 rounded-xl border border-dashed border-slate-700 bg-slate-900/30 py-8 text-slate-500">
          <Film className="h-6 w-6" />
          <p className="text-sm">{t("hangar.best_landings.empty")}</p>
          <p className="text-[11px]">{t("hangar.best_landings.enable_hint")}</p>
        </div>
      )}
    </div>
  );
}

/** Tarjeta de un clip REAL grabado (play / favorito / borrar). */
function ClipCard({ clip, onReload }: { clip: LandingClip; onReload: () => void }) {
  const push = useToastStore((s) => s.push);
  const grade = clip.grade ?? "";
  const color = GRADE_COLOR[grade] ?? "#38bdf8";

  const play = () => {
    api
      .openLocalPath(clip.path)
      .catch((e) =>
        push({ kind: "error", title: t("hangar.clip.play_error"), message: String(e) }),
      );
  };
  const toggleFav = async () => {
    try {
      await api.setLandingFavorite(clip.id, !clip.favorite);
      onReload();
    } catch (e) {
      push({ kind: "error", title: String(e) });
    }
  };
  const remove = async () => {
    try {
      await api.deleteLandingClip(clip.id);
      onReload();
    } catch (e) {
      push({ kind: "error", title: String(e) });
    }
  };

  return (
    <div className="group w-52 shrink-0 overflow-hidden rounded-xl border border-slate-800 bg-slate-950/40">
      <button
        onClick={play}
        className="relative flex h-24 w-full items-center justify-center bg-gradient-to-br from-slate-700/50 via-slate-900 to-slate-950"
      >
        <span className="font-mono text-2xl font-bold text-slate-300/70">
          {clip.airportIcao ?? "✈"}
        </span>
        <span className="absolute inset-0 flex items-center justify-center bg-slate-950/0 transition-colors group-hover:bg-slate-950/40">
          <Play className="h-7 w-7 text-white opacity-0 transition-opacity group-hover:opacity-90" />
        </span>
        {clip.fpm != null && (
          <span
            className="absolute right-2 top-2 rounded-md px-1.5 py-0.5 text-[10px] font-semibold text-slate-950"
            style={{ backgroundColor: color }}
          >
            {clip.fpm} fpm
          </span>
        )}
        {clip.isTest && (
          <span className="absolute left-2 top-2 rounded bg-slate-950/70 px-1.5 py-0.5 text-[9px] text-slate-400">
            {t("hangar.clip.test")}
          </span>
        )}
      </button>
      <div className="flex items-center justify-between gap-1 p-2.5">
        <div className="min-w-0">
          <div className="truncate text-xs font-medium text-slate-200">
            {clip.airportName ?? clip.airportIcao ?? t("hangar.clip.test")}
          </div>
          <div className="truncate text-[10px] text-slate-500">
            {fmtDate(clip.recordedAt)}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <button
            onClick={toggleFav}
            title={t("hangar.clip.favorite")}
            className={`rounded p-1 hover:bg-slate-800 ${
              clip.favorite ? "text-amber-400" : "text-slate-500"
            }`}
          >
            <Star
              className="h-3.5 w-3.5"
              fill={clip.favorite ? "currentColor" : "none"}
            />
          </button>
          <button
            onClick={remove}
            title={t("hangar.clip.delete")}
            className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-rose-400"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}

function LandingCard({ l }: { l: HangarLanding }) {
  const color = GRADE_COLOR[l.grade] ?? "#38bdf8";
  const gradeLabel =
    l.grade === "butter"
      ? t("hangar.fpm.butter")
      : l.grade === "acceptable"
        ? t("hangar.fpm.acceptable")
        : t("hangar.fpm.hard");
  return (
    <div className="w-52 shrink-0 overflow-hidden rounded-xl border border-slate-800 bg-slate-950/40">
      <div className="relative flex h-24 items-center justify-center bg-gradient-to-br from-slate-700/50 via-slate-900 to-slate-950">
        <span className="font-mono text-2xl font-bold text-slate-300/80">
          {l.icao ?? "—"}
        </span>
        <span
          className="absolute right-2 top-2 rounded-md px-1.5 py-0.5 text-[10px] font-semibold text-slate-950"
          style={{ backgroundColor: color }}
        >
          {l.fpm} fpm · {gradeLabel}
        </span>
        <span className="absolute bottom-2 right-2 inline-flex items-center gap-1 rounded bg-slate-950/70 px-1.5 py-0.5 text-[9px] text-slate-400">
          <Film className="h-2.5 w-2.5" />
          {t("hangar.clip_soon")}
        </span>
      </div>
      <div className="p-2.5">
        <div className="truncate text-xs font-medium text-slate-200">
          {l.airportName ?? l.icao ?? "—"}
        </div>
        <div className="mt-0.5 flex items-center justify-between text-[10px] text-slate-500">
          <span>{fmtDate(l.date)}</span>
          <span>{l.model ? cleanAtcType(l.model) ?? l.model : ""}</span>
        </div>
      </div>
    </div>
  );
}

function fmtDate(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
