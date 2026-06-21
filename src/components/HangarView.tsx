import { useEffect, useMemo, useState } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  ResponsiveContainer,
  LabelList,
  Treemap,
  RadarChart,
  PolarGrid,
  PolarAngleAxis,
  PolarRadiusAxis,
  Radar,
  Tooltip,
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
      <div className="mt-5 grid grid-cols-1 items-start gap-4 lg:grid-cols-[320px_1fr]">
        {/* Flota */}
        <section className="self-start rounded-2xl border border-slate-800 bg-slate-900/40 p-3">
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
            {fleet.map((ac, i) => (
              <FleetRow
                key={ac.key}
                rank={search.trim() ? 0 : i + 1}
                ac={ac}
                active={ac.key === selected?.key}
                // (v6.1) Re-click sobre el avión activo → cierra y vuelve al resumen.
                onClick={() =>
                  setSelectedKey((k) => (k === ac.key ? null : ac.key))
                }
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

  const aircraftData = data.topAircraftTypes.map((a) => ({
    name: cleanAtcType(a.label) ?? a.label,
    size: a.count,
  }));
  const airlineData = data.topAirlines.map((a) => ({ name: a.label, size: a.count }));
  const destData = data.topDestinations.map((d) => ({
    name: d.icao,
    size: d.visits,
  }));
  const regionData = topRegions.map((r) => ({ label: r.label, count: r.count }));

  return (
    <section className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4">
      <h2 className="mb-1 text-lg font-bold text-slate-100">
        {t("hangar.overview.heading")}
      </h2>
      <p className="mb-4 text-xs text-slate-500">{t("hangar.overview.hint")}</p>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <TreemapCard
          icon={<Plane className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.aircraft")}
          data={aircraftData}
        />
        <TreemapCard
          icon={<Building2 className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.airlines")}
          data={airlineData}
        />
        <TreemapCard
          icon={<MapPin className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.destinations")}
          data={destData}
        />
        <RadarCard
          icon={<Globe2 className="h-4 w-4 text-brand-400" />}
          title={t("hangar.overview.regions")}
          data={regionData}
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

const TREE_COLORS = [
  "#0ea5e9", "#22c55e", "#f59e0b", "#a855f7", "#ef4444", "#14b8a6",
  "#eab308", "#6366f1", "#ec4899", "#84cc16", "#fb7185", "#64748b",
];

function CardShell({
  icon,
  title,
  empty,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  empty: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      <h3 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
        {icon}
        {title}
      </h3>
      {empty ? (
        <div className="py-10 text-center text-xs text-slate-600">
          {t("hangar.overview.no_data")}
        </div>
      ) : (
        children
      )}
    </div>
  );
}

function TreeLabel(props: {
  x?: number; y?: number; width?: number; height?: number; index?: number; name?: string;
}) {
  const { x = 0, y = 0, width = 0, height = 0, index = 0, name = "" } = props;
  const c = TREE_COLORS[index % TREE_COLORS.length];
  return (
    <g>
      <rect x={x} y={y} width={width} height={height} fill={c} stroke="#0b1220" strokeWidth={2} />
      {width > 46 && height > 24 && (
        <text x={x + 7} y={y + 18} fill="#0b1220" fontSize={12} fontWeight={600}>
          {name.length > Math.floor(width / 8) ? name.slice(0, Math.floor(width / 8) - 1) + "…" : name}
        </text>
      )}
    </g>
  );
}

function TreeTip({ active, payload }: { active?: boolean; payload?: Array<{ payload: { name: string; size: number } }> }) {
  if (!active || !payload?.length) return null;
  const p = payload[0].payload;
  return (
    <div className="rounded-lg border border-slate-700 bg-slate-950/95 px-2.5 py-1.5 text-xs shadow-xl">
      <div className="font-semibold text-slate-100">{p.name}</div>
      <div className="text-slate-400">{p.size} {t("hangar.flights_short")}</div>
    </div>
  );
}

function TreemapCard({
  icon, title, data,
}: {
  icon: React.ReactNode; title: string; data: Array<{ name: string; size: number }>;
}) {
  return (
    <CardShell icon={icon} title={title} empty={data.length === 0}>
      <div className="h-52 w-full">
        <ResponsiveContainer width="100%" height="100%">
          <Treemap data={data} dataKey="size" stroke="#0b1220" isAnimationActive={false} content={<TreeLabel />}>
            <Tooltip content={<TreeTip />} />
          </Treemap>
        </ResponsiveContainer>
      </div>
    </CardShell>
  );
}

function RadarCard({
  icon, title, data,
}: {
  icon: React.ReactNode; title: string; data: Array<{ label: string; count: number }>;
}) {
  return (
    <CardShell icon={icon} title={title} empty={data.length < 3}>
      <div className="h-52 w-full">
        <ResponsiveContainer width="100%" height="100%">
          <RadarChart data={data} outerRadius="70%">
            <PolarGrid stroke="#334155" />
            <PolarAngleAxis dataKey="label" tick={{ fill: "#94a3b8", fontSize: 11 }} />
            <PolarRadiusAxis tick={{ fill: "#64748b", fontSize: 9 }} stroke="#334155" angle={45} />
            <Radar dataKey="count" stroke="#38bdf8" fill="#38bdf8" fillOpacity={0.45} isAnimationActive={false} />
          </RadarChart>
        </ResponsiveContainer>
      </div>
    </CardShell>
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

// ── (v6.1 #24) Tren de aterrizaje por avión (config REAL) ────────────────────

/**
 * Especificación del tren PRINCIPAL por tipo, con los EJES reales:
 *   · 737 / A320 / regional: 1 eje (2 ruedas).
 *   · 757/767/787/747/A330/A350-900: bogie de 2 ejes (4 ruedas).
 *   · 777 / A350-1000 / A380: bogie de 3 ejes (6 ruedas).
 *   · GA: 1 eje, 1 rueda.
 * `maker` cambia el estilo (Boeing: cubos pulidos, sin compuerta; Airbus:
 * compuerta + cubo gris con agujeros).
 */
interface GearSpec {
  axles: number; // nº de ejes en tándem (1 = pata simple; 2/3 = bogie)
  perAxle: number; // ruedas por eje (2 normal, 1 en GA)
  maker: "boeing" | "airbus" | "regional" | "ga";
  big: boolean; // widebody (pata más gruesa)
  door: boolean; // compuerta visible (típico Airbus)
}

function gearSpec(model: string | null): GearSpec {
  const m = (model ?? "").toUpperCase();
  const has = (...ks: string[]) => ks.some((k) => m.includes(k));

  // Aviación general / ligeros — 1 eje, 1 rueda.
  if (has("C150", "C152", "C172", "C182", "C208", "DA40", "DA42", "SR2", "SR20", "SR22", "TBM", "M20", "P28", "PA-", "DV20", "DA62", "CIRRUS", "CESSNA"))
    return { axles: 1, perAxle: 1, maker: "ga", big: false, door: false };
  // Regionales / turbohélice — 1 eje, 2 ruedas.
  if (has("CRJ", "ERJ", "E135", "E145", "E170", "E175", "E190", "E195", "DH8", "DHC", "Q400", "ATR", "AT4", "AT7", "SF34", "SAAB", "B190", "JS", "D328", "RJ85", "RJ1"))
    return { axles: 1, perAxle: 2, maker: "regional", big: false, door: false };

  // Bogie de 3 ejes (6 ruedas).
  if (has("777", "B777", "A35K", "A350-1000", "351", "A380", "388", "389"))
    return { axles: 3, perAxle: 2, maker: m.includes("A3") ? "airbus" : "boeing", big: true, door: m.includes("A3") };
  // Bogie de 2 ejes (4 ruedas).
  if (has("747", "B747", "787", "B787", "767", "B767", "757", "B757", "MD11", "DC10", "L1011"))
    return { axles: 2, perAxle: 2, maker: "boeing", big: true, door: false };
  if (has("A330", "A340", "A350", "A359", "A300", "A310", "A33", "A34"))
    return { axles: 2, perAxle: 2, maker: "airbus", big: true, door: true };

  // Narrowbody Airbus (A320 family) — 1 eje, compuerta, cubo gris.
  if (has("A318", "A319", "A320", "A321", "A32", "A20N", "A21N", "A19N", "NEO"))
    return { axles: 1, perAxle: 2, maker: "airbus", big: false, door: true };
  // Narrowbody Boeing (737/717/MD80) — 1 eje, sin compuerta, cubo pulido.
  if (has("737", "B737", "73", "717", "MD8", "MD9", "DC9", "BBJ"))
    return { axles: 1, perAxle: 2, maker: "boeing", big: false, door: false };

  // Por defecto: 1 eje, 2 ruedas.
  return { axles: 1, perAxle: 2, maker: "boeing", big: false, door: false };
}

const MAKER_LABEL: Record<GearSpec["maker"], string> = {
  boeing: "Boeing",
  airbus: "Airbus",
  regional: "Regional",
  ga: "GA",
};

/** Salud por ZONA del tren (0-100). El rojo parpadea donde hay daño. */
interface GearDamage {
  strut: number; // amortiguador / pata
  wheels: number; // frenos y neumáticos
  braces: number; // soportes / hidráulico
}

const DAMAGE_TH = 78; // por debajo de esto, la zona parpadea en rojo
const redOf = (h: number) => (h >= 70 ? "#f87171" : h >= 45 ? "#ef4444" : "#dc2626");

/**
 * (v6.1 #24) Tren de aterrizaje en VISTA FRONTAL estilo diagrama técnico (como
 * la referencia): horquilla de drag-braces arriba, fitting central, pata-oleo
 * con bandas, pistón, eje y dos ruedas. Las ZONAS con daño (según mantenimiento)
 * parpadean en ROJO: oleo (strut), frenos/neumáticos (wheels) o soportes (braces).
 */
function LandingGear({ model, damage }: { model: string | null; damage: GearDamage }) {
  const s = gearSpec(model);
  const cx = 120;
  const big = s.big;
  const wr = big ? 36 : s.maker === "ga" ? 30 : 34;
  const track = s.perAxle === 1 ? 0 : wr * 0.96;
  const wy = 198;
  const hub = s.maker === "airbus" ? "url(#lgHubA)" : "url(#lgHubB)";

  const strutHurt = damage.strut < DAMAGE_TH;
  const wheelsHurt = damage.wheels < DAMAGE_TH;
  const bracesHurt = damage.braces < DAMAGE_TH;
  const redW = damage.strut >= 70 ? 16 : damage.strut >= 45 ? 21 : 26;

  const Limb = ({ x1, y1, x2, y2, w }: { x1: number; y1: number; x2: number; y2: number; w: number }) => (
    <>
      <line x1={x1} y1={y1} x2={x2} y2={y2} stroke="#1e293b" strokeWidth={w + 2.5} strokeLinecap="round" />
      <line x1={x1} y1={y1} x2={x2} y2={y2} stroke="url(#lgStrut)" strokeWidth={w} strokeLinecap="round" />
    </>
  );

  const Wheel = ({ x }: { x: number }) => {
    const detail =
      s.maker === "airbus"
        ? Array.from({ length: 5 }, (_, i) => {
            const a = (i / 5) * Math.PI * 2 - Math.PI / 2;
            return <circle key={i} cx={x + Math.cos(a) * wr * 0.3} cy={wy + Math.sin(a) * wr * 0.3} r={wr * 0.1} fill="#0f172a" opacity="0.85" />;
          })
        : Array.from({ length: 8 }, (_, i) => {
            const a = (i / 8) * Math.PI * 2;
            return <circle key={i} cx={x + Math.cos(a) * wr * 0.34} cy={wy + Math.sin(a) * wr * 0.34} r={wr * 0.05} fill="#1e293b" />;
          });
    return (
      <g>
        <circle cx={x} cy={wy} r={wr} fill="url(#lgTire)" stroke="#000" strokeWidth="1.5" />
        <circle cx={x} cy={wy} r={wr * 0.74} fill="none" stroke="#0a0a0a" strokeWidth={wr * 0.12} />
        <circle cx={x} cy={wy} r={wr * 0.52} fill={hub} stroke="#1e293b" strokeWidth="1" />
        {detail}
        <circle cx={x} cy={wy} r={wr * 0.16} fill="url(#lgChrome)" stroke="#334155" strokeWidth="0.8" />
        <ellipse cx={x - wr * 0.28} cy={wy - wr * 0.34} rx={wr * 0.22} ry={wr * 0.11} fill="#fff" opacity="0.12" />
        {/* Daño en frenos/neumáticos → anillo rojo parpadeante en el cubo */}
        {wheelsHurt && (
          <circle
            className="gear-blink"
            cx={x}
            cy={wy}
            r={wr * 0.62}
            fill="none"
            stroke={redOf(damage.wheels)}
            strokeWidth={wr * 0.16}
          />
        )}
      </g>
    );
  };

  return (
    <svg viewBox="0 0 240 252" width="208" height="218" xmlns="http://www.w3.org/2000/svg" className="select-none" aria-hidden>
      <defs>
        <linearGradient id="lgStrut" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#5b6675" />
          <stop offset="0.4" stopColor="#c7d0da" />
          <stop offset="0.5" stopColor="#eef3f8" />
          <stop offset="0.6" stopColor="#c7d0da" />
          <stop offset="1" stopColor="#5b6675" />
        </linearGradient>
        <linearGradient id="lgChrome" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#64748b" />
          <stop offset="0.5" stopColor="#fbfdff" />
          <stop offset="1" stopColor="#64748b" />
        </linearGradient>
        <radialGradient id="lgTire" cx="0.42" cy="0.36" r="0.72">
          <stop offset="0" stopColor="#3a3a3a" />
          <stop offset="0.55" stopColor="#161616" />
          <stop offset="1" stopColor="#000000" />
        </radialGradient>
        <radialGradient id="lgHubB" cx="0.4" cy="0.34" r="0.78">
          <stop offset="0" stopColor="#f8fafc" />
          <stop offset="0.5" stopColor="#c2ccd6" />
          <stop offset="1" stopColor="#5b6675" />
        </radialGradient>
        <radialGradient id="lgHubA" cx="0.4" cy="0.34" r="0.78">
          <stop offset="0" stopColor="#cfd6dd" />
          <stop offset="0.5" stopColor="#94a3b1" />
          <stop offset="1" stopColor="#3f4b59" />
        </radialGradient>
        <filter id="lgShadow" x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="4" />
        </filter>
      </defs>

      <ellipse cx={cx} cy={wy + wr + 7} rx={track + wr + 10} ry="8" fill="#000" opacity="0.4" filter="url(#lgShadow)" />

      {/* Horquilla de drag-braces (A-frame), simétrica */}
      <Limb x1={cx - 6} y1={86} x2={56} y2={42} w={9} />
      <Limb x1={cx + 6} y1={86} x2={184} y2={42} w={9} />
      <Limb x1={cx - 4} y1={92} x2={92} y2={50} w={7} />
      <Limb x1={cx + 4} y1={92} x2={148} y2={50} w={7} />
      {[[56, 42], [184, 42], [92, 50], [148, 50]].map(([fx, fy], i) => (
        <g key={i}>
          <line x1={fx} y1={fy} x2={fx - 7} y2={fy - 11} stroke="#475569" strokeWidth="3.5" strokeLinecap="round" />
          <line x1={fx} y1={fy} x2={fx + 7} y2={fy - 11} stroke="#475569" strokeWidth="3.5" strokeLinecap="round" />
        </g>
      ))}
      {/* Daño en soportes/hidráulico → parpadeo rojo sobre las braces */}
      {bracesHurt && (
        <g className="gear-blink" style={{ mixBlendMode: "screen" }}>
          <line x1={cx - 6} y1={86} x2={56} y2={42} stroke={redOf(damage.braces)} strokeWidth="9" strokeLinecap="round" />
          <line x1={cx + 6} y1={86} x2={184} y2={42} stroke={redOf(damage.braces)} strokeWidth="9" strokeLinecap="round" />
        </g>
      )}

      {/* Fitting central (trunnion) */}
      <rect x={cx - 19} y="78" width="38" height="34" rx="6" fill="url(#lgStrut)" stroke="#1e293b" strokeWidth="1" />
      <rect x={cx - 19} y="90" width="38" height="8" fill="#94a3b8" opacity="0.5" />

      {/* Compuerta (Airbus) */}
      {s.door && (
        <rect x={cx + 22} y="78" width="15" height="40" rx="3" fill="#cbd5e1" stroke="#64748b" strokeWidth="1" opacity="0.85" />
      )}

      {/* Oleo (pata) con bandas + pistón */}
      <rect x={cx - 13} y="110" width="26" height="48" rx="6" fill="url(#lgStrut)" stroke="#1e293b" strokeWidth="0.8" />
      {[120, 132, 144].map((y) => (
        <rect key={y} x={cx - 13} y={y} width="26" height="3" fill="#1e293b" opacity="0.35" />
      ))}
      <rect x={cx - 9} y="156" width="18" height="30" rx="4" fill="url(#lgChrome)" stroke="#334155" strokeWidth="0.6" />

      {/* Daño en el oleo/amortiguador → parpadeo rojo en la pata */}
      {strutHurt && (
        <rect
          className="gear-blink"
          x={cx - redW / 2}
          y="108"
          width={redW}
          height="80"
          rx={redW / 2}
          fill={redOf(damage.strut)}
          style={{ mixBlendMode: "screen" }}
        />
      )}

      {/* Eje + ruedas */}
      <rect x={cx - track - 6} y={wy - 7} width={track * 2 + 12} height="14" rx="7" fill="url(#lgStrut)" stroke="#1e293b" strokeWidth="0.6" />
      {s.perAxle === 1 ? (
        <Wheel x={cx} />
      ) : (
        <>
          <Wheel x={cx - track} />
          <Wheel x={cx + track} />
        </>
      )}
    </svg>
  );
}

function GearHealthCard({ ac }: { ac: HangarAircraft }) {
  const pct = ac.healthPct;
  // (v6.1) Daño por ZONA del tren a partir del mantenimiento sintético
  // (determinista por matrícula → cada avión "ha sufrido" en sitios distintos).
  const mx = useMemo(() => deriveMaintenance(ac), [ac]);
  const compHealth = (k: string) =>
    mx.components.find((c) => c.key === k)?.healthPct ?? 100;
  const damage = {
    strut: ac.healthPct, // el tren propiamente dicho (salud del tren)
    wheels: compHealth("brakes"),
    braces: compHealth("hydraulics"),
  };
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
        <LandingGear model={ac.model} damage={damage} />
        <span className="text-[10px] uppercase tracking-wide text-slate-600">
          {(() => {
            const s = gearSpec(ac.model);
            const wheels = s.axles * s.perAxle;
            return `${MAKER_LABEL[s.maker]} · ${t("hangar.gear.wheels_n", { n: String(wheels) })}${s.axles > 1 ? ` · bogie ${s.axles} ejes` : " · 1 eje"}`;
          })()}
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
