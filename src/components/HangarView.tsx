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
  AlertTriangle,
  Play,
  Star,
  Trash2,
  MapPin,
  Building2,
  Globe2,
  LayoutGrid,
  Wrench,
  FileText,
  Printer,
  Clock,
  X,
} from "lucide-react";
import type {
  HangarAnalytics,
  HangarAircraft,
  HangarLanding,
  HangarCount,
  LandingClip,
} from "../lib/types";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";
import { AirlineLogo } from "./AirlineLogo";
import { cleanAtcType } from "../lib/aircraft";
import { airportRegion } from "../lib/oaciRegion";
import {
  deriveMaintenance,
  type MaintenanceData,
  type MaintenanceReport,
} from "../lib/hangarMaintenance";
import { useAircraftPhoto } from "./FlightBookView";
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

/** (v6.1 #27) ¿El clip pertenece a este avión? Por matrícula si la hay; si no,
 *  por modelo. */
function clipMatchesAircraft(c: LandingClip, ac: HangarAircraft): boolean {
  const reg = ac.registration?.trim().toUpperCase();
  if (reg && c.registration) {
    return c.registration.trim().toUpperCase() === reg;
  }
  const model = ac.model?.trim().toUpperCase();
  if (model && c.model) return c.model.trim().toUpperCase() === model;
  return false;
}

/** (v6.1 #28) URL reproducible del clip local para la miniatura `<video>`.
 *  Usa el protocolo asset de Tauri; en demo (web) no aplica. */
function clipSrc(path: string): string | null {
  if (!isTauri) return null;
  try {
    return convertFileSrc(path);
  } catch {
    return null;
  }
}

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
            recorded={clips}
            onReloadClips={reloadClips}
          />
        ) : (
          <FleetOverview
            data={data}
            clips={clips}
            onReloadClips={reloadClips}
          />
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

function FleetOverview({
  data,
  clips,
  onReloadClips,
}: {
  data: HangarAnalytics;
  clips: LandingClip[];
  onReloadClips: () => void;
}) {
  // (v6.1 #27) Top 10 de los mejores aterrizajes de TODA la flota: clips reales
  // ordenados por FPM más fino; si no hay clips, los sintéticos del backend.
  const topClips = useMemo(
    () =>
      [...clips]
        .sort((a, b) => (b.fpm ?? -9999) - (a.fpm ?? -9999))
        .slice(0, 10),
    [clips],
  );
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

      {/* (v6.1 #27) Top 10 mejores aterrizajes de toda la flota. */}
      <div className="mt-2">
        <BestLandings
          recorded={topClips}
          synthetic={data.bestLandings}
          onReload={onReloadClips}
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
  recorded,
  onReloadClips,
}: {
  ac: HangarAircraft;
  recorded: LandingClip[];
  onReloadClips: () => void;
}) {
  const [tab, setTab] = useState<(typeof TABS)[number]>("overview");
  const push = useToastStore((s) => s.push);
  // (v6.1 #26) Foto del avión (planespotters por matrícula) de fondo del banner.
  const photo = useAircraftPhoto(ac.registration ?? null);
  // (v6.1 #31 #32) Mantenimiento sintético derivado del uso (determinista).
  const mx = useMemo(() => deriveMaintenance(ac), [ac]);
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
        {/* (v6.1 #26) Foto real del avión de fondo (si hay matrícula con
            foto en planespotters); overlay oscuro para legibilidad. */}
        {photo && (
          <>
            <img
              src={photo}
              alt={ac.registration ?? ""}
              className="absolute inset-0 h-full w-full object-cover"
            />
            <div className="absolute inset-0 bg-gradient-to-r from-slate-950/95 via-slate-950/70 to-slate-950/30" />
            <span className="pointer-events-none absolute bottom-1.5 right-2 z-10 rounded bg-slate-950/60 px-1.5 py-0.5 text-[8px] font-medium uppercase tracking-wider text-slate-300/80 backdrop-blur-sm">
              planespotters.net
            </span>
          </>
        )}
        <div
          className={`relative flex items-end gap-4 p-5 ${
            photo
              ? ""
              : "bg-gradient-to-br from-slate-800 via-slate-900 to-brand-900/40"
          }`}
        >
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
          {!photo && (
            <PlaneLanding className="h-16 w-16 shrink-0 text-slate-700/60" />
          )}
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
        {tab === "overview" && (
          <>
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              <GearHealthCard ac={ac} />
              <FpmTrendCard ac={ac} />
            </div>
            <BestLandings
              recorded={recorded.filter((c) => clipMatchesAircraft(c, ac))}
              synthetic={ac.recentLandings}
              onReload={onReloadClips}
            />
          </>
        )}
        {tab === "performance" && <PerformanceTab ac={ac} mx={mx} />}
        {tab === "maintenance" && <MaintenanceTab mx={mx} />}
        {tab === "flights" && <FlightsTab ac={ac} />}
        {tab === "history" && <HistoryTab mx={mx} />}
        {tab === "documents" && <DocumentsTab ac={ac} mx={mx} />}
      </div>
    </section>
  );
}

// ── (v6.1 #31 #32) Pestañas: Performance / Maintenance / Flights / Documents ─

function hours(s: number): string {
  return `${Math.round(s / 3600).toLocaleString()} h`;
}
function money(n: number): string {
  return `$${Math.round(n).toLocaleString()}`;
}
const STATUS_COLOR: Record<string, string> = {
  good: "#3fbf78",
  watch: "#f59e0b",
  alert: "#f43f5e",
};

function StatTile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      <div className="text-[10px] uppercase tracking-wide text-slate-500">{label}</div>
      <div className="mt-1 text-xl font-bold text-slate-100">{value}</div>
      {sub && <div className="text-[11px] text-slate-500">{sub}</div>}
    </div>
  );
}

function PerformanceTab({ ac, mx }: { ac: HangarAircraft; mx: MaintenanceData }) {
  const butter = ac.recentLandings.filter((l) => l.grade === "butter").length;
  const rate =
    ac.recentLandings.length > 0
      ? Math.round((butter / ac.recentLandings.length) * 100)
      : 0;
  const avgLeg =
    ac.flights > 0 ? Math.round(ac.totalNm / ac.flights) : 0;
  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
      <StatTile label={t("hangar.kpi.miles")} value={`${Math.round(ac.totalNm).toLocaleString()} nm`} />
      <StatTile label={t("hangar.perf.hours")} value={hours(ac.totalTimeS)} />
      <StatTile label={t("hangar.perf.cycles")} value={mx.cycles.toLocaleString()} />
      <StatTile label={t("hangar.perf.avg_leg")} value={`${avgLeg.toLocaleString()} nm`} />
      <StatTile label={t("hangar.fpm.avg")} value={ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)} fpm` : "—"} />
      <StatTile label={t("hangar.perf.worst")} value={ac.worstLandingFpm != null ? `${ac.worstLandingFpm} fpm` : "—"} />
      <StatTile label={t("hangar.perf.butter_rate")} value={`${rate}%`} sub={`${butter}/${ac.recentLandings.length}`} />
      <StatTile label={t("hangar.perf.hard")} value={String(ac.hardLandings)} />
    </div>
  );
}

function ComponentBar({ label, health, status, nextDueHours }: {
  label: string; health: number; status: string; nextDueHours: number;
}) {
  const color = STATUS_COLOR[status] ?? "#38bdf8";
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-slate-200">{label}</span>
        <span className="text-xs font-semibold" style={{ color }}>{health}%</span>
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-slate-800">
        <div className="h-full rounded-full" style={{ width: `${health}%`, backgroundColor: color }} />
      </div>
      <div className="mt-1.5 text-[10px] text-slate-500">
        {nextDueHours <= 0
          ? t("hangar.mx.overdue")
          : t("hangar.mx.next_in", { n: nextDueHours.toLocaleString() })}
      </div>
    </div>
  );
}

function MaintenanceTab({ mx }: { mx: MaintenanceData }) {
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatTile label={t("hangar.mx.total_hours")} value={`${mx.hours.toLocaleString()} h`} />
        <StatTile label={t("hangar.perf.cycles")} value={mx.cycles.toLocaleString()} />
        <StatTile label={t("hangar.mx.next_service")} value={`${Math.max(0, mx.nextServiceHours).toLocaleString()} h`} />
        <StatTile label={t("hangar.mx.lifetime_cost")} value={money(mx.lifetimeCost)} />
      </div>
      <div>
        <h4 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
          <Wrench className="h-4 w-4 text-brand-400" />
          {t("hangar.mx.components")}
        </h4>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {mx.components.map((c) => (
            <ComponentBar key={c.key} label={c.label} health={c.healthPct} status={c.status} nextDueHours={c.nextDueHours} />
          ))}
        </div>
      </div>
    </div>
  );
}

function FlightsTab({ ac }: { ac: HangarAircraft }) {
  const flights = [...ac.recentLandings].reverse(); // reciente→antiguo
  if (flights.length === 0) {
    return <div className="py-10 text-center text-sm text-slate-500">{t("hangar.flights.empty")}</div>;
  }
  return (
    <div className="overflow-hidden rounded-xl border border-slate-800">
      <table className="w-full text-left text-xs">
        <thead className="bg-slate-900/60 text-[10px] uppercase tracking-wide text-slate-500">
          <tr>
            <th className="px-3 py-2">{t("hangar.flights.date")}</th>
            <th className="px-3 py-2">{t("hangar.flights.dest")}</th>
            <th className="px-3 py-2 text-right">{t("hangar.flights.time")}</th>
            <th className="px-3 py-2 text-right">FPM</th>
          </tr>
        </thead>
        <tbody>
          {flights.map((l, i) => (
            <tr key={i} className="border-t border-slate-800/60">
              <td className="px-3 py-2 text-slate-300">{fmtDate(l.date)}</td>
              <td className="px-3 py-2 text-slate-200">
                {l.airportName ?? l.icao ?? "—"}
              </td>
              <td className="px-3 py-2 text-right text-slate-400">
                {l.flightTimeS ? hours(l.flightTimeS) : "—"}
              </td>
              <td className="px-3 py-2 text-right font-semibold" style={{ color: GRADE_COLOR[l.grade] ?? "#94a3b8" }}>
                {l.fpm}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function HistoryTab({ mx }: { mx: MaintenanceData }) {
  return (
    <div className="space-y-2">
      {mx.reports.map((r) => (
        <div key={r.id} className="flex items-start gap-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-xs font-bold text-white" style={{ backgroundColor: r.shop.color }}>
            {r.shop.code}
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-semibold text-slate-100">{r.type} · {r.shop.name}</span>
              <span className="shrink-0 text-[11px] text-slate-500">{fmtDate(r.date)}</span>
            </div>
            <p className="mt-0.5 truncate text-[11px] text-slate-400">{r.summary}</p>
            <div className="mt-1 flex items-center gap-3 text-[10px] text-slate-500">
              <span><Clock className="mr-0.5 inline h-3 w-3" />{r.hoursAtService.toLocaleString()} h</span>
              <span className="font-semibold text-slate-300">{money(r.totalCost)}</span>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function DocumentsTab({ ac, mx }: { ac: HangarAircraft; mx: MaintenanceData }) {
  const [report, setReport] = useState<MaintenanceReport | null>(null);
  return (
    <div>
      <p className="mb-3 text-[11px] text-slate-500">{t("hangar.docs.hint")}</p>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {mx.reports.map((r) => (
          <button
            key={r.id}
            onClick={() => setReport(r)}
            className="group flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-left hover:border-brand-500/40"
          >
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-slate-900 text-brand-300 ring-1 ring-slate-800 group-hover:text-brand-200">
              <FileText className="h-5 w-5" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-xs font-semibold text-slate-100">
                {t("hangar.docs.report")} · {r.type}
              </div>
              <div className="truncate text-[11px] text-slate-500">
                {r.shop.name} · {fmtDate(r.date)}
              </div>
              <div className="text-[11px] font-semibold text-slate-300">{money(r.totalCost)}</div>
            </div>
          </button>
        ))}
      </div>
      {report && (
        <ReportModal ac={ac} report={report} hoursTotal={mx.hours} onClose={() => setReport(null)} />
      )}
    </div>
  );
}

/** Reporte de mantenimiento imprimible (→ PDF con el diálogo del sistema). */
function ReportModal({ ac, report, hoursTotal, onClose }: {
  ac: HangarAircraft; report: MaintenanceReport; hoursTotal: number; onClose: () => void;
}) {
  const r = report;
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : "—";
  return (
    <div className="fixed inset-0 z-[120] flex items-start justify-center overflow-y-auto bg-slate-950/80 p-6 backdrop-blur-sm print:bg-white print:p-0" onClick={onClose}>
      <div
        onClick={(e) => e.stopPropagation()}
        className="hangar-print-area w-full max-w-3xl rounded-2xl bg-white p-8 text-slate-900 shadow-2xl print:max-w-none print:rounded-none print:shadow-none"
      >
        {/* Encabezado del taller */}
        <div className="flex items-start justify-between border-b-2 pb-4" style={{ borderColor: r.shop.color }}>
          <div className="flex items-center gap-3">
            <span className="flex h-12 w-12 items-center justify-center rounded-lg text-lg font-black text-white" style={{ backgroundColor: r.shop.color }}>
              {r.shop.code}
            </span>
            <div>
              <div className="text-lg font-bold">{r.shop.name}</div>
              <div className="text-xs text-slate-500">{t("hangar.docs.work_order")} #{r.id.slice(-6).toUpperCase()}</div>
            </div>
          </div>
          <div className="text-right text-xs text-slate-500">
            <div className="font-semibold text-slate-700">{t("hangar.docs.report")}</div>
            <div>{fmtDate(r.date)}</div>
          </div>
        </div>

        {/* Datos del avión */}
        <div className="mt-4 grid grid-cols-2 gap-x-8 gap-y-1 text-sm sm:grid-cols-4">
          <Field2 k={t("hangar.docs.reg")} v={ac.registration ?? "—"} />
          <Field2 k={t("hangar.docs.model")} v={model} />
          <Field2 k={t("hangar.docs.airframe_hours")} v={`${r.hoursAtService.toLocaleString()} h`} />
          <Field2 k={t("hangar.perf.cycles")} v={r.cyclesAtService.toLocaleString()} />
          <Field2 k={t("hangar.docs.check_type")} v={r.type} />
          <Field2 k={t("hangar.docs.mechanic")} v={r.mechanic} />
          <Field2 k={t("hangar.docs.total_hours")} v={`${hoursTotal.toLocaleString()} h`} />
          <Field2 k={t("hangar.docs.next_due")} v={`${fmtDate(r.nextDueDate)} (${r.nextDueHours} h)`} />
        </div>

        {/* Resumen */}
        <div className="mt-4">
          <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">{t("hangar.docs.summary")}</div>
          <p className="mt-1 text-sm">{r.summary}</p>
        </div>

        {/* Tabla de piezas */}
        <table className="mt-4 w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-300 text-xs uppercase tracking-wide text-slate-500">
              <th className="py-1.5">{t("hangar.docs.part")}</th>
              <th className="py-1.5">{t("hangar.docs.pn")}</th>
              <th className="py-1.5 text-center">{t("hangar.docs.qty")}</th>
              <th className="py-1.5">{t("hangar.docs.reason")}</th>
              <th className="py-1.5 text-right">{t("hangar.docs.cost")}</th>
            </tr>
          </thead>
          <tbody>
            {r.parts.map((p, i) => (
              <tr key={i} className="border-b border-slate-200">
                <td className="py-1.5 font-medium">{p.name}</td>
                <td className="py-1.5 font-mono text-xs text-slate-600">{p.partNumber}</td>
                <td className="py-1.5 text-center">{p.qty}</td>
                <td className="py-1.5 text-xs text-slate-600">{p.reason}</td>
                <td className="py-1.5 text-right">{money(p.cost * p.qty)}</td>
              </tr>
            ))}
          </tbody>
        </table>

        {/* Totales */}
        <div className="mt-4 ml-auto w-56 space-y-1 text-sm">
          <Row k={t("hangar.docs.parts")} v={money(r.partsCost)} />
          <Row k={t("hangar.docs.labor")} v={money(r.laborCost)} />
          <div className="flex justify-between border-t border-slate-300 pt-1 font-bold">
            <span>{t("hangar.docs.total")}</span>
            <span>{money(r.totalCost)}</span>
          </div>
        </div>

        <p className="mt-6 text-[10px] text-slate-400">{t("hangar.docs.disclaimer")}</p>

        {/* Acciones (no se imprimen) */}
        <div className="mt-6 flex justify-end gap-2 print:hidden">
          <button onClick={onClose} className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-sm text-slate-600 hover:bg-slate-100">
            <X className="h-4 w-4" />
            {t("common.close")}
          </button>
          <button onClick={() => window.print()} className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-white hover:bg-brand-400">
            <Printer className="h-4 w-4" />
            {t("hangar.docs.print")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Field2({ k, v }: { k: string; v: string }) {
  return (
    <div>
      <div className="text-[10px] uppercase tracking-wide text-slate-400">{k}</div>
      <div className="font-medium">{v}</div>
    </div>
  );
}
function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between text-slate-600">
      <span>{k}</span>
      <span className="font-medium text-slate-800">{v}</span>
    </div>
  );
}

// ── (v6.1 #24) Tren de aterrizaje ilustrado según el tipo de avión ──────────

type GearVariant = "wide" | "narrow" | "regional" | "light";

/** Clasifica el tren por el modelo/tipo. Heurística por tokens del nombre. */
function gearVariant(model: string | null): GearVariant {
  const m = (model ?? "").toUpperCase();
  const has = (...ks: string[]) => ks.some((k) => m.includes(k));
  // Widebody → bogies de 4+ ruedas en tándem.
  if (
    has(
      "747", "777", "787", "767", "A330", "A340", "A350", "A380", "MD11",
      "MD-11", "DC10", "DC-10", "L1011", "IL96", "A300", "A310",
    )
  ) {
    return "wide";
  }
  // Regional / turboprop → tren ligero de 2 ruedas.
  if (
    has(
      "CRJ", "ERJ", "E135", "E145", "E170", "E175", "DH8", "DHC", "AT4", "AT7",
      "ATR", "Q400", "SF34", "SAAB", "BEH", "B190", "JS", "D328",
    )
  ) {
    return "regional";
  }
  // Aviación general / ligeros → una rueda pequeña por pata.
  if (
    has(
      "C150", "C152", "C172", "C182", "C208", "PA", "DA40", "DA42", "SR2",
      "TBM", "BE", "M20", "P28", "DV20", "CIRRUS", "CESSNA",
    )
  ) {
    return "light";
  }
  // Por defecto: narrowbody (737/A320/etc).
  return "narrow";
}

/**
 * (v6.1 #24) Tren de aterrizaje VISTO DE FRENTE, SVG realista (gradientes
 * metálicos, neumáticos con banda roja y cubo con tornillos, pistón cromado,
 * líneas hidráulicas y sombra). La configuración de ruedas cambia con el tipo:
 *   widebody = bogie de 4 · narrowbody = 2 · regional = 2 · GA = 1.
 */
function LandingGear({ model }: { model: string | null }) {
  const v = gearVariant(model);
  const axleY = 150;
  const wheels: { cx: number; r: number }[] =
    v === "wide"
      ? [
          { cx: 58, r: 30 },
          { cx: 98, r: 30 },
          { cx: 142, r: 30 },
          { cx: 182, r: 30 },
        ]
      : v === "narrow"
        ? [
            { cx: 76, r: 42 },
            { cx: 164, r: 42 },
          ]
        : v === "regional"
          ? [
              { cx: 84, r: 34 },
              { cx: 156, r: 34 },
            ]
          : [{ cx: 120, r: 46 }];
  const minCx = Math.min(...wheels.map((w) => w.cx));
  const maxCx = Math.max(...wheels.map((w) => w.cx));

  const Tire = ({ cx, r }: { cx: number; r: number }) => {
    const bolts = Array.from({ length: 8 }, (_, i) => {
      const a = (i / 8) * Math.PI * 2;
      return { x: cx + Math.cos(a) * r * 0.32, y: axleY + Math.sin(a) * r * 0.32 };
    });
    return (
      <g>
        {/* Goma */}
        <circle cx={cx} cy={axleY} r={r} fill="url(#lgTire)" stroke="#000" strokeWidth="1.5" />
        {/* Banda roja del flanco */}
        <circle cx={cx} cy={axleY} r={r * 0.78} fill="none" stroke="#a01818" strokeWidth={r * 0.16} />
        <circle cx={cx} cy={axleY} r={r * 0.7} fill="none" stroke="#000" strokeWidth="1" opacity="0.5" />
        {/* Cubo */}
        <circle cx={cx} cy={axleY} r={r * 0.5} fill="url(#lgHub)" stroke="#1e293b" strokeWidth="1" />
        {bolts.map((b, i) => (
          <circle key={i} cx={b.x} cy={b.y} r={r * 0.05} fill="#1e293b" />
        ))}
        {/* Tapa central */}
        <circle cx={cx} cy={axleY} r={r * 0.16} fill="url(#lgChrome)" stroke="#334155" strokeWidth="0.8" />
        {/* Brillo */}
        <ellipse cx={cx - r * 0.28} cy={axleY - r * 0.34} rx={r * 0.22} ry={r * 0.12} fill="#ffffff" opacity="0.12" />
      </g>
    );
  };

  return (
    <svg
      viewBox="0 0 240 200"
      width="210"
      height="175"
      xmlns="http://www.w3.org/2000/svg"
      className="select-none"
      aria-hidden
    >
      <defs>
        <linearGradient id="lgStrut" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#26303c" />
          <stop offset="0.32" stopColor="#8a9bad" />
          <stop offset="0.5" stopColor="#f3f7fb" />
          <stop offset="0.68" stopColor="#8a9bad" />
          <stop offset="1" stopColor="#26303c" />
        </linearGradient>
        <linearGradient id="lgChrome" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#5b6b7d" />
          <stop offset="0.5" stopColor="#fbfdff" />
          <stop offset="1" stopColor="#5b6b7d" />
        </linearGradient>
        <radialGradient id="lgTire" cx="0.42" cy="0.38" r="0.7">
          <stop offset="0" stopColor="#3b3b3b" />
          <stop offset="0.55" stopColor="#161616" />
          <stop offset="1" stopColor="#000000" />
        </radialGradient>
        <radialGradient id="lgHub" cx="0.4" cy="0.36" r="0.75">
          <stop offset="0" stopColor="#f1f5f9" />
          <stop offset="0.5" stopColor="#aab6c4" />
          <stop offset="1" stopColor="#3f4b5a" />
        </radialGradient>
        <filter id="lgShadow" x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="4" />
        </filter>
      </defs>

      {/* Sombra en el suelo */}
      <ellipse
        cx={(minCx + maxCx) / 2}
        cy={axleY + wheels[0].r + 12}
        rx={(maxCx - minCx) / 2 + wheels[0].r + 8}
        ry="9"
        fill="#000"
        opacity="0.35"
        filter="url(#lgShadow)"
      />

      {/* Eje entre ruedas (detrás) */}
      {wheels.length > 1 && (
        <rect x={minCx} y={axleY - 7} width={maxCx - minCx} height="14" rx="7" fill="url(#lgStrut)" />
      )}

      {/* Pata principal (oleo) + pistón cromado */}
      <rect x="108" y="14" width="24" height="86" rx="9" fill="url(#lgStrut)" stroke="#1e293b" strokeWidth="0.6" />
      <rect x="113" y="92" width="14" height="60" rx="6" fill="url(#lgChrome)" stroke="#334155" strokeWidth="0.6" />
      {/* Anillo prensaestopas */}
      <rect x="106" y="96" width="28" height="7" rx="3" fill="url(#lgStrut)" stroke="#1e293b" strokeWidth="0.5" />
      {/* Tijera (torque link) */}
      <path d="M120 104 L132 122 L120 140" fill="none" stroke="#cbd5e1" strokeWidth="3.5" strokeLinejoin="round" />
      <circle cx="132" cy="122" r="2.6" fill="#475569" />
      {/* Líneas hidráulicas */}
      <path d="M104 30 q-8 40 4 70" fill="none" stroke="#1f2937" strokeWidth="3" />
      <path d="M104 30 q-8 40 4 70" fill="none" stroke="#b8860b" strokeWidth="1.4" />
      {/* Soportes superiores */}
      <path d="M120 24 L150 40" stroke="url(#lgStrut)" strokeWidth="7" strokeLinecap="round" />
      <path d="M120 24 L90 40" stroke="url(#lgStrut)" strokeWidth="7" strokeLinecap="round" />

      {/* Ruedas */}
      {wheels.map((w, i) => (
        <Tire key={i} cx={w.cx} r={w.r} />
      ))}
    </svg>
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
      <div className="mt-4 flex flex-col items-center justify-center gap-2">
        <LandingGear model={ac.model} />
        <span className="text-[10px] uppercase tracking-wide text-slate-600">
          {t(`hangar.gear.type.${gearVariant(ac.model)}`)}
        </span>
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
      <h3 className="mb-2 flex items-center gap-1.5 text-base font-semibold text-slate-100">
        <Gauge className="h-4 w-4 text-brand-400" />
        {t("hangar.fpm.title", { n: String(n) })}
        <Info className="h-3 w-3 text-slate-600" />
      </h3>
      {n === 0 ? (
        <div className="flex h-40 items-center justify-center text-sm text-slate-600">
          {t("hangar.fpm.no_data")}
        </div>
      ) : (
        <div className="h-52 w-full">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart
              data={chartData}
              margin={{ top: 18, right: 14, left: -4, bottom: 0 }}
            >
              <CartesianGrid stroke="#1e293b" vertical={false} />
              <XAxis
                dataKey="label"
                tick={{ fill: "#94a3b8", fontSize: 13 }}
                axisLine={{ stroke: "#1e293b" }}
                tickLine={false}
              />
              <YAxis
                tick={{ fill: "#94a3b8", fontSize: 13 }}
                axisLine={false}
                tickLine={false}
                width={44}
                domain={["dataMin - 50", 50]}
              />
              <Line
                type="monotone"
                dataKey="fpm"
                stroke="#38bdf8"
                strokeWidth={2.5}
                dot={<FpmDot />}
                isAnimationActive={false}
              >
                <LabelList
                  dataKey="fpm"
                  position="top"
                  fontSize={12}
                  fontWeight={600}
                  fill="#e2e8f0"
                />
              </Line>
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
      {/* Leyenda */}
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-slate-800 pt-3">
        <div className="flex flex-wrap gap-3 text-[12px]">
          <Legend color={GRADE_COLOR.butter} label={t("hangar.fpm.butter")} n={counts.butter ?? 0} />
          <Legend color={GRADE_COLOR.acceptable} label={t("hangar.fpm.acceptable")} n={counts.acceptable ?? 0} />
          <Legend color={GRADE_COLOR.hard} label={t("hangar.fpm.hard")} n={counts.hard ?? 0} />
        </div>
        <div className="text-right">
          <div className="text-[10px] uppercase tracking-wide text-slate-500">
            {t("hangar.fpm.avg")}
          </div>
          <div className="text-base font-semibold text-slate-100">{avg}</div>
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
          <span className="text-[11px] text-slate-500">
            {recorded.length} {t("hangar.flights_short")}
          </span>
        )}
      </div>

      {hasClips ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          {recorded.map((c) => (
            <ClipCard key={c.id} clip={c} onReload={onReload} />
          ))}
        </div>
      ) : synthetic.length > 0 ? (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
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

  const src = clipSrc(clip.path);
  return (
    <div className="group w-full overflow-hidden rounded-xl border border-slate-800 bg-slate-950/40">
      <button
        onClick={play}
        className="relative flex aspect-video w-full items-center justify-center overflow-hidden bg-gradient-to-br from-slate-700/50 via-slate-900 to-slate-950"
      >
        {/* (v6.1 #28) Miniatura REAL: el <video> con preload=metadata pinta
            el primer frame del clip (como el explorador de Windows). Sin
            asset/src cae al ICAO. */}
        {src ? (
          <video
            src={src}
            muted
            playsInline
            preload="metadata"
            className="absolute inset-0 h-full w-full object-cover"
          />
        ) : (
          <span className="font-mono text-2xl font-bold text-slate-300/70">
            {clip.airportIcao ?? "✈"}
          </span>
        )}
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
    <div className="w-full overflow-hidden rounded-xl border border-slate-800 bg-slate-950/40">
      <div className="relative flex aspect-video items-center justify-center bg-gradient-to-br from-slate-700/50 via-slate-900 to-slate-950">
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
