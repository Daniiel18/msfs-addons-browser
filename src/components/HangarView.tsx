import { Fragment, useEffect, useMemo, useState } from "react";
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
  AreaChart,
  Area,
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
  Loader2,
  Play,
  Star,
  Trash2,
  MapPin,
  Building2,
  Globe2,
  Wrench,
  FileText,
  Printer,
  X,
  Flame,
  Thermometer,
  ChevronRight,
} from "lucide-react";
import type {
  HangarAnalytics,
  HangarAircraft,
  HangarLanding,
  HangarCount,
  LandingClip,
  FlightLogEntry,
  MaintRecord,
  TechLogAircraft,
} from "../lib/types";
import { deriveTelemetry, type FlightTelemetry } from "../lib/flightTelemetry";
import worldLand from "../lib/worldLand.json";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";
import { TechLogBadge, TechLogPanel, TechLogSummaryCard, useTechLogAircraft } from "./TechLogPanel";
import { normalizeTechLogReg, useTechLogStore } from "../stores/useTechLogStore";
import { maintComponentLabel } from "../lib/maintLabels";
import { useAppStore } from "../stores/useAppStore";
import { AirlineLogo } from "./AirlineLogo";
import { cleanAtcType } from "../lib/aircraft";
import { airportRegion } from "../lib/oaciRegion";
import { resolveIata } from "../lib/airlineCodes";
import { deriveMaintenance, type MaintenanceReport } from "../lib/hangarMaintenance";
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
  const [page, setPage] = useState(0); // (v6.1) paginación de la flota (10/pág)

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

  // (v6.1 #33) Buscando → filtra TODA la flota; si no, 10 por página.
  const PER_PAGE = 10;
  const searching = search.trim() !== "";
  const pageCount = data ? Math.max(1, Math.ceil(data.aircraft.length / PER_PAGE)) : 1;
  const fleet = useMemo(() => {
    if (!data) return [];
    const q = search.trim().toLowerCase();
    if (q) {
      return data.aircraft.filter((a) =>
        [a.registration, a.model, a.airlineName]
          .filter(Boolean)
          .some((s) => s!.toLowerCase().includes(q)),
      );
    }
    return data.aircraft.slice(page * PER_PAGE, page * PER_PAGE + PER_PAGE);
  }, [data, search, page]);
  // Al buscar, reseteamos a la primera página al limpiar la búsqueda.
  useEffect(() => {
    if (searching) setPage(0);
  }, [searching]);

  // (v6.1 #29) null = vista de resumen de flota (sin avión seleccionado).
  const selected = data?.aircraft.find((a) => a.key === selectedKey) ?? null;
  // (19/10) Preflight / avisos piden abrir el mantenimiento de una matrícula.
  const hangarFocusRequest = useAppStore((s) => s.hangarFocusRequest);
  const [tabRequest, setTabRequest] = useState<{ tab: "maintenance"; n: number } | null>(null);
  useEffect(() => {
    if (!hangarFocusRequest || !data) return;
    const want = normalizeTechLogReg(hangarFocusRequest);
    const ac = data.aircraft.find((a) => a.registration && normalizeTechLogReg(a.registration) === want);
    useAppStore.getState().clearHangarFocusRequest();
    if (!ac) return;
    // (21/10) Que se vea resaltado en la lista: su página, sin búsqueda.
    setSearch("");
    setPage(Math.floor(data.aircraft.indexOf(ac) / PER_PAGE));
    setSelectedKey(ac.key);
    setTabRequest({ tab: "maintenance", n: Date.now() });
  }, [hangarFocusRequest, data]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-sf-muted">
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
      <div className="flex h-full flex-col items-center justify-center gap-2 text-sf-muted">
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
          <h1 className="text-xl font-semibold tracking-tight text-sf-text">
            {t("hangar.heading")}
          </h1>
          <p className="mt-0.5 text-sm text-sf-muted">{t("hangar.subheading")}</p>
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
      <div className="mt-5 grid grid-cols-1 items-start gap-4 md:grid-cols-[280px_1fr] lg:grid-cols-[320px_1fr]">
        {/* Flota */}
        <section className="self-start rounded-2xl border border-sf-border bg-sf-surface p-3">
          <h2 className="mb-2 flex items-center gap-1.5 px-1 text-sm font-semibold text-sf-text">
            {t("hangar.fleet")}
            <Info className="h-3.5 w-3.5 text-sf-muted/70" />
          </h2>
          <div className="mb-3 flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-sf-muted" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("hangar.search")}
                className="w-full rounded-lg border border-sf-border bg-sf-bg/60 py-1.5 pl-8 pr-2 text-xs text-sf-text placeholder:text-sf-muted/70 focus:border-brand-500/50 focus:outline-none"
              />
            </div>
            <button className="rounded-lg border border-sf-border bg-sf-bg/60 p-1.5 text-sf-muted hover:text-sf-text">
              <SlidersHorizontal className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="space-y-1.5">
            {fleet.map((ac, i) => (
              <FleetRow
                key={ac.key}
                rank={searching ? 0 : page * PER_PAGE + i + 1}
                ac={ac}
                active={ac.key === selected?.key}
                // (v6.1) Re-click sobre el avión activo → cierra y vuelve al resumen.
                onClick={() =>
                  setSelectedKey((k) => (k === ac.key ? null : ac.key))
                }
              />
            ))}
            {fleet.length === 0 && (
              <p className="px-1 py-3 text-center text-[11px] text-sf-muted/70">
                {t("hangar.search_empty")}
              </p>
            )}
          </div>

          {/* (v6.1) Paginación de la flota (cuando hay más de 10 y no se busca) */}
          {!searching && pageCount > 1 && (
            <div className="mt-3 flex items-center justify-between border-t border-sf-border pt-2.5">
              <button
                onClick={() => setPage((p) => Math.max(0, p - 1))}
                disabled={page === 0}
                className="inline-flex items-center gap-1 rounded-lg border border-sf-border bg-sf-bg/60 px-2 py-1 text-[11px] text-sf-text/90 hover:border-brand-500/40 disabled:opacity-40"
              >
                <ChevronRight className="h-3.5 w-3.5 rotate-180" />
                {t("hangar.page.prev")}
              </button>
              <span className="text-[11px] text-sf-muted">
                {t("hangar.page.of", { a: String(page + 1), b: String(pageCount) })}
              </span>
              <button
                onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
                disabled={page >= pageCount - 1}
                className="inline-flex items-center gap-1 rounded-lg border border-sf-border bg-sf-bg/60 px-2 py-1 text-[11px] text-sf-text/90 hover:border-brand-500/40 disabled:opacity-40"
              >
                {t("hangar.page.next")}
                <ChevronRight className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
        </section>

        {/* Detalle del avión, o resumen de flota si no hay selección. */}
        {selected ? (
          <AircraftDetail
            ac={selected}
            recorded={clips}
            onReloadClips={reloadClips}
            tabRequest={tabRequest}
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
    <div className="flex items-center gap-2.5 rounded-xl border border-sf-border bg-sf-surface px-3 py-2">
      <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15 text-brand-300">
        {icon}
      </div>
      <div>
        <div className="text-[11px] uppercase tracking-wide text-sf-muted">
          {label}
        </div>
        <div className="text-sm font-semibold text-sf-text">{value}</div>
        <div className="text-[11px] text-sf-muted">{t("hangar.all_aircraft")}</div>
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
  const airlineData = data.topAirlines.map((a) => ({
    name: a.label,
    size: a.count,
    img: airlineLogoUrl(a.code, a.label),
    code: a.code,
  }));
  const destData = data.topDestinations.map((d) => ({
    name: d.icao,
    size: d.visits,
    img: flagUrl(d.icao),
  }));
  const regionData = topRegions.map((r) => ({ label: r.label, count: r.count }));

  return (
    <section className="rounded-2xl border border-sf-border bg-sf-surface p-4">
      <h2 className="mb-1 text-base font-semibold text-sf-text">
        {t("hangar.overview.heading")}
      </h2>
      <p className="mb-4 text-xs text-sf-muted">{t("hangar.overview.hint")}</p>
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
          hint={t("hangar.overview.airlines_hint")}
          onSelect={(item) =>
            useAppStore.getState().openEconomy(item.code ?? item.name)
          }
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
  hint,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  empty: boolean;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-sf-border bg-sf-bg/40 p-3">
      <h3 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-sf-text">
        {icon}
        {title}
        {hint && (
          <span className="ml-auto text-[10px] font-normal text-sf-muted">
            {hint}
          </span>
        )}
      </h3>
      {empty ? (
        <div className="py-10 text-center text-xs text-sf-muted/70">
          {t("hangar.overview.no_data")}
        </div>
      ) : (
        children
      )}
    </div>
  );
}

/** Logo de aerolínea (avs.io) desde ICAO/nombre, o null si no hay IATA. */
function airlineLogoUrl(code: string | null, name: string): string | null {
  const iata = resolveIata((code ?? "").toUpperCase(), name);
  return iata ? `https://pics.avs.io/al_square/120/120/${iata}.png` : null;
}
/** Bandera del país (flagcdn) desde el ICAO del aeropuerto, o null. */
function flagUrl(icao: string): string | null {
  const code = airportRegion(icao).code;
  const iso = code === "USA" ? "us" : /^[A-Z]{2}$/.test(code) ? code.toLowerCase() : null;
  return iso ? `https://flagcdn.com/120x90/${iso}.png` : null;
}

/**
 * (v6.1) Celda del treemap: CUADRADA (las 4 esquinas del dashboard las redondea
 * el contenedor). Logo/bandera GRANDE y CENTRADO, escalado al tamaño de la
 * celda (responsive), con el nombre debajo, también centrado.
 */
function TreeLabel(props: {
  x?: number; y?: number; width?: number; height?: number; index?: number;
  name?: string; img?: string | null; code?: string | null;
  onSelect?: (item: { name: string; code: string | null }) => void;
}) {
  const { x = 0, y = 0, width = 0, height = 0, index = 0, name = "", img, code, onSelect } = props;
  const c = TREE_COLORS[index % TREE_COLORS.length];
  const cx = x + width / 2;
  const cy = y + height / 2;
  const hasImg = !!img && width > 44 && height > 44;
  // Imagen responsive: hasta ~52% del lado menor de la celda.
  const isz = hasImg ? Math.min(width, height) * 0.52 : 0;
  const fontSize = Math.max(10, Math.min(14, Math.min(width, height) * 0.13));
  const maxChars = Math.max(3, Math.floor((width - 10) / (fontSize * 0.6)));
  const label = name.length > maxChars ? name.slice(0, maxChars - 1) + "…" : name;
  const textY = hasImg ? cy + isz / 2 + fontSize : cy + fontSize * 0.36;
  const clickable = !!onSelect && !!name;
  return (
    <g
      onClick={clickable ? () => onSelect!({ name, code: code ?? null }) : undefined}
      style={clickable ? { cursor: "pointer" } : undefined}
    >
      {/* Cuadrada: el "gap" entre celdas lo da el stroke; sin rx. */}
      <rect x={x} y={y} width={width} height={height} fill={c} stroke="#0b1220" strokeWidth={2.5} />
      {hasImg && (
        <image
          href={img!}
          x={cx - isz / 2}
          y={cy - isz / 2 - fontSize * 0.6}
          width={isz}
          height={isz * 0.78}
          preserveAspectRatio="xMidYMid meet"
        />
      )}
      {width > 40 && height > 22 && (
        <text x={cx} y={textY} textAnchor="middle" fill="#0b1220" fontSize={fontSize} fontWeight={500}>
          {label}
        </text>
      )}
    </g>
  );
}

function TreeTip({ active, payload }: { active?: boolean; payload?: Array<{ payload: { name: string; size: number } }> }) {
  if (!active || !payload?.length) return null;
  const p = payload[0].payload;
  return (
    <div className="rounded-lg border border-sf-border bg-sf-bg/95 px-2.5 py-1.5 text-xs shadow-xl">
      <div className="font-semibold text-sf-text">{p.name}</div>
      <div className="text-sf-muted">{p.size} {t("hangar.flights_short")}</div>
    </div>
  );
}

function TreemapCard({
  icon, title, data, hint, onSelect,
}: {
  icon: React.ReactNode;
  title: string;
  data: Array<{ name: string; size: number; code?: string | null }>;
  /** Pista bajo el título (ej. "clic para ver finanzas"). */
  hint?: string;
  /** Si se pasa, cada celda es clicable y dispara esto con su item. */
  onSelect?: (item: { name: string; code: string | null }) => void;
}) {
  return (
    <CardShell icon={icon} title={title} empty={data.length === 0} hint={hint}>
      <div className="h-52 w-full overflow-hidden rounded-lg">
        <ResponsiveContainer width="100%" height="100%">
          <Treemap
            data={data}
            dataKey="size"
            stroke="#0b1220"
            isAnimationActive={false}
            content={<TreeLabel onSelect={onSelect} />}
          >
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
          : "border-transparent bg-sf-bg/40 hover:bg-sf-raised/70"
      }`}
    >
      <span className="w-4 shrink-0 text-center text-xs font-semibold text-sf-muted">
        {rank > 0 ? rank : ""}
      </span>
      <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={32} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-xs font-semibold text-sf-text">
          {ac.airlineName ?? model ?? t("hangar.unknown_model")}
        </div>
        <div className="truncate text-[11px] text-sf-muted">{model ?? "—"}</div>
        {ac.registration && (
          <div className="truncate font-mono text-[10px] text-sf-muted/70">
            {ac.registration}
          </div>
        )}
      </div>
      {ac.flights === 0 ? (
        // (21/10) Avión que sólo existe en Fenix/PMDG TechLog (sin vuelos aquí).
        <span className="shrink-0 rounded-full bg-sf-raised px-2 py-0.5 text-[10px] font-medium text-sf-muted ring-1 ring-sf-border">
          {t("hangar.techlog_only")}
        </span>
      ) : (
        <div className="shrink-0 text-right">
          <div className="text-xs font-semibold text-sf-text">
            {Math.round(ac.totalNm).toLocaleString()} nm
          </div>
          <div className="text-[10px] text-sf-muted">
            {ac.flights} {t("hangar.flights_short")}
          </div>
        </div>
      )}
      <MoreVertical className="h-3.5 w-3.5 shrink-0 text-sf-muted/70" />
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
  tabRequest,
}: {
  ac: HangarAircraft;
  recorded: LandingClip[];
  onReloadClips: () => void;
  tabRequest?: { tab: "maintenance"; n: number } | null;
}) {
  const [tab, setTab] = useState<(typeof TABS)[number]>("overview");
  useEffect(() => {
    if (tabRequest) setTab(tabRequest.tab);
  }, [tabRequest]);
  // (v6.1 #26) Foto del avión (planespotters por matrícula) de fondo del banner.
  const photo = useAircraftPhoto(ac.registration ?? null);
  // (v6.1) Servicios REALES cobrados (maintenance_log, los mismos que
  // Finanzas): Historial y Documentos. (20/10) Etapa 4: el mantenimiento
  // inventado (deriveMaintenance) y los componentes del sistema viejo ya no
  // se usan aquí: todo sale del Tech Log (Fenix/PMDG TechLog o el de SimFleet).
  const tlEpoch = useTechLogStore((s) => s.epoch);
  const [realHistory, setRealHistory] = useState<MaintRecord[]>([]);
  useEffect(() => {
    let alive = true;
    if (ac.registration) {
      api
        .aircraftMaintenanceHistory(ac.registration)
        .then((h) => alive && setRealHistory(h))
        .catch(() => {});
    } else {
      setRealHistory([]);
    }
    return () => {
      alive = false;
    };
  }, [ac.registration, tlEpoch]);
  const [reportOpen, setReportOpen] = useState(false);
  // (19/10) Etapa 2: si este avión tiene Fenix/PMDG TechLog, TechLog manda
  // (resumen, pestaña Mantenimiento y chip del banner salen de TechLog).
  const tlManaged = useTechLogStore((s) => s.isManaged(ac.registration));
  // (19/10) Etapa 3: el resto de aviones usa el Tech Log de SimFleet.
  const techlog = useTechLogAircraft(ac.registration, tlManaged, ac.model);
  const tl = techlog.data;
  // (20/10) Estado general = vida media de las piezas; color según despacho.
  const health = tl ? tl.condition.avgRemainingPct : null;
  const healthBar = !tl
    ? "bg-slate-500"
    : tl.dispatch.status === "NO_GO"
      ? "bg-rose-400"
      : tl.dispatch.status === "GO_MEL"
        ? "bg-amber-400"
        : "bg-emerald-400";
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : null;
  // (v8.3) Ubicación con traslados: en curso → "En traslado a X · llega HH:MM".
  const ferryEnroute =
    ac.ferryToIcao && ac.ferryArrivesAt && Date.parse(ac.ferryArrivesAt) > Date.now();
  const location = ferryEnroute
    ? t("hangar.ferry_enroute", {
        to: ac.ferryToIcao as string,
        time: new Date(ac.ferryArrivesAt as string).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
        }),
      })
    : ac.lastAirportName && ac.lastIcao
      ? `${ac.lastAirportName} (${ac.lastIcao})`
      : ac.lastIcao ?? null;

  return (
    <section className="flex flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface lg:self-stretch">
      {/* Banner */}
      <div className="relative overflow-hidden">
        <div className="absolute right-3 top-3 z-10 flex">
          <button
            onClick={() => setReportOpen(true)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-sf-border bg-slate-950/70 px-3 py-1.5 text-xs text-sf-text backdrop-blur hover:border-brand-500/40"
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
            <span className="pointer-events-none absolute bottom-1.5 right-2 z-10 rounded bg-sf-bg/60 px-1.5 py-0.5 text-[8px] font-medium uppercase tracking-wider text-sf-text/90/80 backdrop-blur-sm">
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
              {tl && <TechLogBadge data={tl} />}
            </div>
            <div className="mt-1.5 flex items-center gap-2">
              <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={20} />
              <span className="text-sm font-medium text-sf-text">
                {ac.airlineName ?? "—"}
              </span>
            </div>
            <p className="mt-0.5 text-xs text-sf-muted">
              {[model, location].filter(Boolean).join(" · ") || "—"}
            </p>
          </div>
          {!photo && (
            <PlaneLanding className="h-16 w-16 shrink-0 text-sf-muted/70/60" />
          )}
        </div>
      </div>

      {/* (Rediseño F4) KPIs del avión bajo la foto + estado general. */}
      <div className="grid grid-cols-2 gap-px border-b border-sf-border/70 bg-sf-border/50 sm:grid-cols-4">
        <AcKpi label={t("hangar.kpi.flights")} value={ac.flights.toLocaleString()} />
        <AcKpi label={t("hangar.kpi.miles")} value={`${Math.round(ac.totalNm).toLocaleString()} nm`} />
        <AcKpi label={t("hangar.kpi.hours")} value={hours(ac.totalTimeS)} />
        <AcKpi
          label={t("hangar.kpi.avg_fpm")}
          value={ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)} fpm` : "—"}
        />
      </div>
      <div className="flex items-center gap-3 border-b border-sf-border/70 px-4 py-2.5">
        <span className="shrink-0 text-xs text-sf-muted" title={t("hangar.health_hint")}>
          {t("hangar.health_general")}
        </span>
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sf-raised">
          <div
            className={`h-full rounded-full ${healthBar}`}
            style={{ width: `${Math.max(0, Math.min(100, health ?? 0))}%` }}
          />
        </div>
        <span className="shrink-0 text-xs font-semibold tabular-nums text-sf-text">
          {health != null ? `${Math.round(health)}%` : "—"}
          {tl && (
            <span className="ml-1.5 font-mono text-[10px] text-sf-muted">
              {tl.dispatch.status === "NO_GO" ? "NO-GO" : tl.dispatch.status === "GO_MEL" ? "GO · MEL" : "GO"}
            </span>
          )}
        </span>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b border-sf-border px-4">
        {TABS.map((tb) => (
          <button
            key={tb}
            onClick={() => setTab(tb)}
            className={`relative px-3 py-2.5 text-xs font-medium transition-colors ${
              tab === tb
                ? "text-brand-300"
                : "text-sf-muted hover:text-sf-text/90"
            }`}
          >
            {t(`hangar.tab.${tb}`)}
            {tab === tb && (
              <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-brand-400" />
            )}
          </button>
        ))}
      </div>

      <div className="flex-1 p-4">
        {tab === "overview" && (
          <>
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              {tl ? (
                <TechLogSummaryCard data={tl} onOpen={() => setTab("maintenance")} />
              ) : (
                <TechLogPending loading={techlog.loading} />
              )}
              <FpmTrendCard ac={ac} />
            </div>
            <BestLandings
              recorded={recorded.filter((c) => clipMatchesAircraft(c, ac))}
              synthetic={ac.recentLandings}
              onReload={onReloadClips}
            />
          </>
        )}
        {tab === "performance" && <PerformanceTab ac={ac} tl={tl} />}
        {tab === "maintenance" &&
          (tl ? (
            <TechLogPanel data={tl} onReload={() => void techlog.reload()} loading={techlog.loading} />
          ) : (
            <TechLogPending loading={techlog.loading} />
          ))}
        {tab === "flights" && <FlightsTab ac={ac} />}
        {tab === "history" && <HistoryTab realHistory={realHistory} />}
        {tab === "documents" && <DocumentsTab ac={ac} tl={tl} realHistory={realHistory} />}
      </div>

      {reportOpen && (
        <AircraftReportModal ac={ac} tl={tl} onClose={() => setReportOpen(false)} />
      )}
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
/** (Rediseño F4) Celda de la fila de KPIs bajo la foto del avión. */
function AcKpi({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-sf-surface px-4 py-3">
      <div className="truncate text-[11px] uppercase tracking-wide text-sf-muted">{label}</div>
      <div className="mt-0.5 truncate text-lg font-semibold tabular-nums text-sf-text">{value}</div>
    </div>
  );
}

function StatTile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-xl border border-sf-border bg-sf-bg/40 p-3">
      <div className="text-[10px] uppercase tracking-wide text-sf-muted">{label}</div>
      <div className="mt-1 text-xl font-bold text-sf-text">{value}</div>
      {sub && <div className="text-[11px] text-sf-muted">{sub}</div>}
    </div>
  );
}

function PerformanceTab({ ac, tl }: { ac: HangarAircraft; tl: TechLogAircraft | null }) {
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
      <StatTile label={t("hangar.perf.cycles")} value={tl ? Math.round(tl.cycles).toLocaleString() : "—"} />
      <StatTile label={t("hangar.perf.avg_leg")} value={`${avgLeg.toLocaleString()} nm`} />
      <StatTile label={t("hangar.fpm.avg")} value={ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)} fpm` : "—"} />
      <StatTile label={t("hangar.perf.worst")} value={ac.worstLandingFpm != null ? `${ac.worstLandingFpm} fpm` : "—"} />
      <StatTile label={t("hangar.perf.butter_rate")} value={`${rate}%`} sub={`${butter}/${ac.recentLandings.length}`} />
      <StatTile label={t("hangar.perf.hard")} value={String(ac.hardLandings)} />
    </div>
  );
}

/** (20/10) Mientras llega (o si falla) el Tech Log del avión: sin datos inventados. */
function TechLogPending({ loading }: { loading: boolean }) {
  return (
    <div className="flex min-h-[8rem] items-center justify-center gap-2 rounded-xl border border-sf-border bg-sf-bg/40 p-4 text-xs text-sf-muted">
      {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Wrench className="h-4 w-4" />}
      {t(loading ? "hangar.techlog_loading" : "hangar.techlog_unavailable")}
    </div>
  );
}
function fpmGrade(fpm: number | null): string {
  if (fpm == null) return "acceptable";
  return fpm > -150 ? "butter" : fpm >= -300 ? "acceptable" : "hard";
}

function FlightsTab({ ac }: { ac: HangarAircraft }) {
  const entries = useFlightLogStore((s) => s.entries);
  const mx = useMemo(() => deriveMaintenance(ac), [ac]);
  const engineHealth = mx.components.find((c) => c.key === "engine")?.healthPct ?? 100;
  const reg = ac.registration?.toUpperCase() ?? null;
  const model = ac.model?.toUpperCase() ?? null;
  const flights = useMemo(
    () =>
      entries
        .filter((e) =>
          reg
            ? e.aircraftRegistration?.toUpperCase() === reg
            : model
              ? (e.aircraftModel ?? e.aircraftAtcType ?? e.aircraftTitle ?? "").toUpperCase() === model
              : false,
        )
        .slice(0, 40),
    [entries, reg, model],
  );
  const [open, setOpen] = useState<number | null>(null);

  if (flights.length === 0) {
    return <div className="py-10 text-center text-sm text-sf-muted">{t("hangar.flights.empty")}</div>;
  }
  return (
    <div className="overflow-hidden rounded-xl border border-sf-border">
      <table className="w-full text-left text-xs">
        <thead className="bg-sf-surface text-[10px] uppercase tracking-wide text-sf-muted">
          <tr>
            <th className="w-6 px-2 py-2"></th>
            <th className="px-2 py-2">{t("hangar.flights.date")}</th>
            <th className="px-2 py-2">{t("hangar.flights.dest")}</th>
            <th className="px-2 py-2 text-right">{t("hangar.flights.time")}</th>
            <th className="px-3 py-2 text-right">FPM</th>
          </tr>
        </thead>
        <tbody>
          {flights.map((f) => {
            const isOpen = open === f.id;
            return (
              <Fragment key={f.id}>
                <tr
                  onClick={() => setOpen(isOpen ? null : f.id)}
                  className={`cursor-pointer border-t border-sf-border/60 transition-colors hover:bg-sf-raised/70 ${isOpen ? "bg-sf-raised/70" : ""}`}
                >
                  <td className="px-2 py-2 text-sf-muted">
                    <ChevronRight className={`h-3.5 w-3.5 transition-transform ${isOpen ? "rotate-90" : ""}`} />
                  </td>
                  <td className="px-2 py-2 text-sf-text/90">{fmtDate(f.startedAt)}</td>
                  <td className="px-2 py-2 text-sf-text">
                    {f.originIcao ?? "—"} → {f.destinationIcao ?? f.destinationName ?? "—"}
                  </td>
                  <td className="px-2 py-2 text-right text-sf-muted">
                    {f.flightTimeS ? hours(f.flightTimeS) : "—"}
                  </td>
                  <td className="px-3 py-2 text-right font-semibold" style={{ color: GRADE_COLOR[fpmGrade(f.landingFpm)] ?? "#94a3b8" }}>
                    {f.landingFpm ?? "—"}
                  </td>
                </tr>
                {isOpen && (
                  <tr className="border-t border-sf-border/60 bg-sf-bg/50">
                    <td colSpan={5} className="p-0">
                      <FlightDetail flight={f} engineHealth={engineHealth} />
                    </td>
                  </tr>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function FlightDetail({ flight, engineHealth }: { flight: FlightLogEntry; engineHealth: number }) {
  const tel = useMemo(() => deriveTelemetry(flight, engineHealth), [flight, engineHealth]);
  const chartData = tel.series.map((p) => ({
    t: `${p.t}%`,
    EGT1: p.egt1,
    EGT2: p.egt2 || null,
    N1: p.n1,
  }));
  return (
    <div className="grid grid-cols-1 gap-4 p-4 lg:grid-cols-2">
      {/* Métricas + gráfico EGT */}
      <div>
        <div className="grid grid-cols-3 gap-2">
          <StatTile label={t("hangar.fd.cruise_alt")} value={`${Math.round(tel.cruiseAltFt).toLocaleString()} ft`} />
          <StatTile label={t("hangar.fd.cruise_spd")} value={`${tel.cruiseSpeedKt} kt`} />
          <StatTile label={t("hangar.fd.max_spd")} value={`${tel.maxSpeedKt} kt`} />
          <StatTile label={t("hangar.fd.avg_n1")} value={`${tel.avgN1}%`} />
          <StatTile label={t("hangar.fd.max_egt")} value={`${tel.maxEgtC}°C`} />
          <StatTile label={t("hangar.fd.fuel")} value={tel.fuelKg != null ? `${Math.round(tel.fuelKg).toLocaleString()} kg` : "—"} />
        </div>
        <div className="mt-3">
          <h4 className="mb-1 flex items-center gap-1.5 text-xs font-semibold text-sf-text/90">
            <Thermometer className="h-3.5 w-3.5 text-rose-400" />
            {t("hangar.fd.egt_title")}
          </h4>
          <div className="h-40 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartData} margin={{ top: 6, right: 8, left: -10, bottom: 0 }}>
                <defs>
                  <linearGradient id="egtFill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0" stopColor="#f43f5e" stopOpacity={0.5} />
                    <stop offset="1" stopColor="#f43f5e" stopOpacity={0.04} />
                  </linearGradient>
                </defs>
                <CartesianGrid stroke="#1e293b" vertical={false} />
                <XAxis dataKey="t" tick={{ fill: "#64748b", fontSize: 10 }} axisLine={{ stroke: "#1e293b" }} tickLine={false} />
                <YAxis tick={{ fill: "#64748b", fontSize: 10 }} axisLine={false} tickLine={false} width={36} unit="°" />
                <Tooltip contentStyle={{ background: "#020617", border: "1px solid #334155", borderRadius: 8, fontSize: 11 }} />
                <Area type="monotone" dataKey="EGT1" stroke="#f43f5e" strokeWidth={2} fill="url(#egtFill)" isAnimationActive={false} />
                {tel.engines > 1 && (
                  <Area type="monotone" dataKey="EGT2" stroke="#fb923c" strokeWidth={2} fill="none" isAnimationActive={false} />
                )}
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>
      </div>

      {/* Dibujo de los motores con zonas de calor */}
      <div>
        <h4 className="mb-1 flex items-center gap-1.5 text-xs font-semibold text-sf-text/90">
          <Flame className="h-3.5 w-3.5 text-orange-400" />
          {t("hangar.fd.engine_title", { n: String(tel.engines) })}
        </h4>
        <EngineDiagram tel={tel} />
        <p className="mt-1 text-[10px] text-sf-muted">{t("hangar.fd.engine_hint")}</p>
      </div>
    </div>
  );
}

/**
 * (v6.1) Turbofan de alto bypass en CUTAWAY, ligeramente inclinado (cielo→
 * tierra) bajo el ala SEMITRANSPARENTE. Se ven los componentes (fan, compresor,
 * cámara de combustión, turbina, tobera) y la ZONA CALIENTE (cámara+turbina)
 * marcada en rojo/naranja. El tamaño del fan y el nombre del motor cambian con
 * el tipo de avión. Para turbohélices dibuja la hélice.
 */
function EngineDiagram({ tel }: { tel: FlightTelemetry }) {
  const heat = tel.heat / 100;
  const hotColor = heat > 0.75 ? "#dc2626" : heat > 0.5 ? "#ef4444" : "#f97316";
  const cy = 130;
  const fanR = Math.max(20, Math.min(52, 36 * tel.fan));
  const coreR = fanR * 0.4;
  const turboprop = tel.turboprop;

  return (
    <svg viewBox="0 0 380 220" width="100%" className="select-none" aria-hidden>
      <defs>
        <linearGradient id="engMetal" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#cbd5e1" />
          <stop offset="0.5" stopColor="#8a9bad" />
          <stop offset="1" stopColor="#475569" />
        </linearGradient>
        <linearGradient id="engCore" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#94a3b8" />
          <stop offset="1" stopColor="#334155" />
        </linearGradient>
        <radialGradient id="engHot" cx="0.5" cy="0.5" r="0.55">
          <stop offset="0" stopColor="#fde68a" />
          <stop offset="0.35" stopColor={hotColor} stopOpacity="0.95" />
          <stop offset="1" stopColor={hotColor} stopOpacity="0" />
        </radialGradient>
      </defs>

      {/* Ala semitransparente (en ángulo) + pilón */}
      <path d="M24 34 L356 16 L362 50 L40 80 Z" fill="#94a3b8" fillOpacity="0.16" stroke="#94a3b8" strokeOpacity="0.45" strokeWidth="1" />
      <path d={`M150 66 L182 66 L176 ${cy - fanR * 0.95} L156 ${cy - fanR * 0.95} Z`} fill="#64748b" stroke="#334155" strokeWidth="1" />

      {turboprop ? (
        <g>
          {/* Hélice */}
          <g stroke="#1e293b" strokeWidth="1.5" fill="#334155">
            {Array.from({ length: 5 }, (_, i) => {
              const a = (i / 5) * Math.PI * 2 - Math.PI / 2;
              return <ellipse key={i} cx={70 + Math.cos(a) * 4} cy={cy + Math.sin(a) * 44} rx="5" ry="22" transform={`rotate(${(a * 180) / Math.PI + 90} ${70} ${cy})`} />;
            })}
          </g>
          <circle cx="70" cy={cy} r="10" fill="url(#engMetal)" stroke="#1e293b" strokeWidth="1.5" />
          {/* Góndola del turbohélice */}
          <path d={`M86 ${cy - 22} L300 ${cy - 16} Q316 ${cy - 14} 316 ${cy} Q316 ${cy + 14} 300 ${cy + 16} L86 ${cy + 22} Z`} fill="url(#engMetal)" stroke="#1e293b" strokeWidth="1.5" />
          <rect x="250" y={cy - 9} width="56" height="18" rx="9" fill="url(#engHot)" />
        </g>
      ) : (
        <g>
          {/* Góndola translúcida (vemos el interior) */}
          <path
            d={`M60 ${cy - fanR * 1.12}
                Q56 ${cy} 60 ${cy + fanR * 1.12}
                L210 ${cy + fanR * 1.05}
                Q300 ${cy + fanR * 0.96} 300 ${cy + fanR * 0.6}
                L300 ${cy - fanR * 0.6}
                Q300 ${cy - fanR * 0.96} 210 ${cy - fanR * 1.05} Z`}
            fill="#94a3b8" fillOpacity="0.16" stroke="#94a3b8" strokeOpacity="0.55" strokeWidth="1.5"
          />
          {/* Labio del inlet */}
          <path d={`M60 ${cy - fanR * 1.12} Q44 ${cy} 60 ${cy + fanR * 1.12}`} fill="none" stroke="url(#engMetal)" strokeWidth="6" strokeLinecap="round" />

          {/* Spinner + Fan (disco de canto) */}
          <path d={`M50 ${cy} L70 ${cy - fanR} L70 ${cy + fanR} Z`} fill="url(#engMetal)" stroke="#1e293b" strokeWidth="1" />
          <ellipse cx="72" cy={cy} rx="9" ry={fanR} fill="url(#engCore)" stroke="#0b1220" strokeWidth="1.5" />
          {Array.from({ length: 13 }, (_, i) => {
            const yy = cy - fanR + ((2 * fanR) / 12) * i;
            return <line key={i} x1="66" y1={yy} x2="78" y2={yy - 3} stroke="#cbd5e1" strokeWidth="1" opacity="0.8" />;
          })}

          {/* Núcleo: cárter + ejes */}
          <path d={`M100 ${cy - coreR} L255 ${cy - coreR * 0.78} L288 ${cy - coreR * 0.5} L288 ${cy + coreR * 0.5} L255 ${cy + coreR * 0.78} L100 ${cy + coreR} Z`} fill="url(#engCore)" stroke="#1e293b" strokeWidth="1" />
          {/* Compresor (etapas decrecientes) */}
          {Array.from({ length: 7 }, (_, i) => {
            const x = 108 + i * 10;
            const h = coreR * (0.95 - i * 0.07);
            return <line key={i} x1={x} y1={cy - h} x2={x} y2={cy + h} stroke="#e2e8f0" strokeWidth="1.6" opacity="0.85" />;
          })}
          {/* Cámara de combustión (caliente) */}
          <rect x="186" y={cy - coreR * 0.82} width="26" height={coreR * 1.64} rx="5" fill="url(#engHot)" stroke={hotColor} strokeWidth="1" />
          {/* Turbina (etapas calientes) */}
          {Array.from({ length: 4 }, (_, i) => {
            const x = 218 + i * 9;
            const h = coreR * (0.6 + i * 0.06);
            return <line key={i} x1={x} y1={cy - h} x2={x} y2={cy + h} stroke={hotColor} strokeWidth="2.4" opacity="0.95" />;
          })}
          {/* Glow de la zona caliente */}
          <ellipse cx="206" cy={cy} rx="58" ry={coreR * 1.5} fill="url(#engHot)" opacity="0.55" />
          {/* Cono de salida + tobera */}
          <path d={`M288 ${cy - coreR * 0.5} L320 ${cy} L288 ${cy + coreR * 0.5} Z`} fill="url(#engCore)" stroke="#1e293b" strokeWidth="1" />
        </g>
      )}

      {/* Etiqueta del motor */}
      <g transform="translate(20,18)">
        <rect x="0" y="0" width={tel.engineName.length * 6.6 + 16} height="18" rx="9" fill="#0b1220" fillOpacity="0.7" stroke="#334155" />
        <text x="8" y="13" fill="#e2e8f0" fontSize="11" fontWeight="600">{tel.engines}× {tel.engineName}</text>
      </g>
      {/* Etiqueta zona caliente */}
      {!turboprop && (
        <g transform={`translate(160,${cy + fanR * 1.12 + 8})`}>
          <rect x="0" y="0" width="118" height="17" rx="8" fill={hotColor} fillOpacity="0.2" stroke={hotColor} strokeOpacity="0.6" />
          <text x="59" y="12" textAnchor="middle" fill={hotColor} fontSize="10" fontWeight="600">{t("hangar.fd.hot")} · {tel.maxEgtC}°C</text>
        </g>
      )}
    </svg>
  );
}

/** Talleres de mantenimiento (para dar nombre/identidad a los servicios). */
const MAINT_SHOPS = [
  { code: "CAC", name: "Continental AeroCare", color: "#ef4444" },
  { code: "JTS", name: "Jet Technic Services", color: "#0ea5e9" },
  { code: "AMS", name: "AeroMaint Solutions", color: "#22c55e" },
  { code: "GLT", name: "Global Line Technics", color: "#a855f7" },
  { code: "SKM", name: "SkyMech Industries", color: "#f59e0b" },
];
/** Taller estable a partir de una semilla (mismo servicio → mismo taller). */
function maintShopFor(seed: string) {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return MAINT_SHOPS[h % MAINT_SHOPS.length];
}

function HistoryTab({ realHistory }: { realHistory: MaintRecord[] }) {
  if (realHistory.length === 0) {
    return <p className="py-6 text-center text-xs text-sf-muted">{t("hangar.history.empty")}</p>;
  }
  return (
    <div className="space-y-2">
      {/* (v6.1) Servicios REALES — mismo estilo de taller que los reportes
          (sin "done in finance"); el taller se asigna de forma estable. */}
      {realHistory.map((r, i) => {
        const shop = maintShopFor(`${r.component}-${r.servicedAt}`);
        return (
          <div key={`real-${i}`} className="flex items-start gap-3 rounded-xl border border-sf-border bg-sf-bg/40 p-3">
            <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-xs font-bold text-white" style={{ backgroundColor: shop.color }}>
              {shop.code}
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-semibold text-sf-text">
                  {maintComponentLabel(r.component)} · {shop.name}
                </span>
                <span className="shrink-0 text-[11px] text-sf-muted">{fmtDate(r.servicedAt)}</span>
              </div>
              <p className="mt-0.5 truncate text-[11px] text-sf-muted">{t("hangar.history.work")}</p>
              <div className="mt-1 flex items-center gap-3 text-[10px] text-sf-muted">
                <span className="font-semibold text-sf-text/90">{money(r.cost)}</span>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** Un cierre diario = todos los servicios reales de un mismo día. */
interface DailyClose {
  day: string; // YYYY-MM-DD
  records: MaintRecord[];
  total: number;
}

/** Construye un reporte (estilo Continental AeroCare) a partir de TODOS los
 *  servicios reales de un día — si hubo más de uno, salen como líneas/piezas. */
function buildDailyReport(d: DailyClose, tl: TechLogAircraft | null): MaintenanceReport {
  // (20/10) Horas, ciclos y próximo check reales del Tech Log.
  const nextCheckH = tl?.checks
    .map((c) => c.hoursLeft)
    .filter((h): h is number => h != null && h > 0)
    .sort((a, b) => a - b)[0];
  const shop = maintShopFor(d.day);
  const parts = d.records.map((r, i) => ({
    name: maintComponentLabel(r.component),
    partNumber: `${r.component.slice(0, 3).toUpperCase()}-${1000 + i}`,
    qty: 1,
    reason: t("hangar.history.work"),
    cost: Math.round(r.cost),
  }));
  const partsCost = Math.round(d.total * 0.7);
  return {
    id: `daily-${d.day}`,
    date: d.day,
    shop,
    mechanic: "Line Maintenance Crew",
    hoursAtService: Math.round(tl?.flightHours ?? 0),
    cyclesAtService: Math.round(tl?.cycles ?? 0),
    type: d.records.length > 1 ? "Line Check" : maintComponentLabel(d.records[0].component),
    parts,
    laborCost: Math.round(d.total - partsCost),
    partsCost,
    totalCost: Math.round(d.total),
    summary: t("hangar.docs.daily_summary", { n: String(d.records.length) }),
    nextDueHours: Math.round(nextCheckH ?? 0),
    nextDueDate: d.day,
  };
}

function DocumentsTab({ ac, tl, realHistory }: { ac: HangarAircraft; tl: TechLogAircraft | null; realHistory: MaintRecord[] }) {
  const [report, setReport] = useState<MaintenanceReport | null>(null);
  // (v6.1) Agrupa los servicios reales por DÍA → un solo reporte/PDF por día.
  const dailyCloses = useMemo<DailyClose[]>(() => {
    const byDay = new Map<string, MaintRecord[]>();
    for (const r of realHistory) {
      const day = (r.servicedAt ?? "").slice(0, 10);
      if (!byDay.has(day)) byDay.set(day, []);
      byDay.get(day)!.push(r);
    }
    return [...byDay.entries()]
      .map(([day, records]) => ({ day, records, total: records.reduce((s, x) => s + x.cost, 0) }))
      .sort((a, b) => b.day.localeCompare(a.day));
  }, [realHistory]);

  return (
    <div>
      <p className="mb-3 text-[11px] text-sf-muted">{t("hangar.docs.hint")}</p>
      {/* (v6.1) Cierre DIARIO: un PDF por día con todos los servicios hechos. */}
      {dailyCloses.length > 0 && (
        <div className="mb-3 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {dailyCloses.map((d) => (
            <button
              key={d.day}
              onClick={() => setReport(buildDailyReport(d, tl))}
              className="group flex items-center gap-3 rounded-xl border border-emerald-700/40 bg-emerald-950/20 p-3 text-left hover:border-emerald-500/60"
            >
              <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-700/40">
                <FileText className="h-5 w-5" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-xs font-semibold text-sf-text">
                  {t("hangar.docs.daily_close")}
                </div>
                <div className="truncate text-[11px] text-sf-muted">
                  {fmtDate(d.day)} · {t("hangar.docs.n_services", { n: String(d.records.length) })}
                </div>
                <div className="text-[11px] font-semibold text-emerald-300">{money(d.total)}</div>
              </div>
            </button>
          ))}
        </div>
      )}
      {dailyCloses.length === 0 && (
        <p className="py-6 text-center text-xs text-sf-muted">{t("hangar.history.empty")}</p>
      )}
      {report && (
        <ReportModal
          ac={ac}
          report={report}
          hoursTotal={Math.round(tl?.flightHours ?? 0)}
          onClose={() => setReport(null)}
        />
      )}
    </div>
  );
}

/** Reporte de mantenimiento imprimible (→ PDF con el diálogo del sistema). */
function ReportModal({ ac, report, hoursTotal, onClose }: {
  ac: HangarAircraft; report: MaintenanceReport; hoursTotal: number; onClose: () => void;
}) {
  const r = report;
  const push = useToastStore((s) => s.push);
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : "—";
  const doPrint = () => {
    const onAfter = () => {
      push({
        kind: "success",
        title: t("hangar.report.saved"),
        message: t("hangar.report.saved_hint"),
        ttlMs: 6000,
      });
      window.removeEventListener("afterprint", onAfter);
    };
    window.addEventListener("afterprint", onAfter);
    window.print();
  };
  return (
    <div className="hangar-print-overlay fixed inset-0 z-[120] flex items-start justify-center overflow-y-auto bg-slate-950/80 p-6 backdrop-blur-sm" onClick={onClose}>
      <div
        onClick={(e) => e.stopPropagation()}
        className="hangar-print-area w-full max-w-3xl rounded-2xl bg-white p-8 text-slate-900 shadow-2xl"
      >
        {/* Encabezado del taller */}
        <div className="flex items-start justify-between border-b-2 pb-4" style={{ borderColor: r.shop.color }}>
          <div className="flex items-center gap-3">
            <span className="flex h-12 w-12 items-center justify-center rounded-lg text-lg font-black text-white" style={{ backgroundColor: r.shop.color }}>
              {r.shop.code}
            </span>
            <div>
              <div className="text-lg font-bold">{r.shop.name}</div>
              <div className="text-xs text-sf-muted">{t("hangar.docs.work_order")} #{r.id.slice(-6).toUpperCase()}</div>
            </div>
          </div>
          <div className="text-right text-xs text-sf-muted">
            <div className="font-semibold text-sf-muted/70">{t("hangar.docs.report")}</div>
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
          <div className="text-xs font-semibold uppercase tracking-wide text-sf-muted">{t("hangar.docs.summary")}</div>
          <p className="mt-1 text-sm">{r.summary}</p>
        </div>

        {/* Tabla de piezas */}
        <table className="mt-4 w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-300 text-xs uppercase tracking-wide text-sf-muted">
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
                <td className="py-1.5 font-mono text-xs text-sf-muted/70">{p.partNumber}</td>
                <td className="py-1.5 text-center">{p.qty}</td>
                <td className="py-1.5 text-xs text-sf-muted/70">{p.reason}</td>
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

        <p className="mt-6 text-[10px] text-sf-muted">{t("hangar.docs.disclaimer")}</p>

        {/* Acciones (no se imprimen) */}
        <div className="mt-6 flex justify-end gap-2 print:hidden">
          <button onClick={onClose} className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-sm text-sf-muted/70 hover:bg-slate-100">
            <X className="h-4 w-4" />
            {t("common.close")}
          </button>
          <button onClick={doPrint} className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-slate-950 hover:bg-brand-400">
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
      <div className="text-[10px] uppercase tracking-wide text-sf-muted">{k}</div>
      <div className="font-medium">{v}</div>
    </div>
  );
}
function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between text-sf-muted/70">
      <span>{k}</span>
      <span className="font-medium text-slate-800">{v}</span>
    </div>
  );
}

// ── (v6.1) Reporte completo del avión (PDF imprimible) + mapa 2D de rutas ─────

/** Mapa 2D (equirectangular) con las rutas de la matrícula. Ajusta el encuadre
 *  al bounding-box de los vuelos. Para imprimir en el reporte. */
function FlightMapSvg({ flights }: { flights: FlightLogEntry[] }) {
  const pts = flights.flatMap((f) => {
    const a: Array<[number, number]> = [];
    if (f.originLat != null && f.originLon != null) a.push([f.originLat, f.originLon]);
    if (f.destinationLat != null && f.destinationLon != null) a.push([f.destinationLat, f.destinationLon]);
    return a;
  });
  const W = 640;
  const H = 300;
  if (pts.length === 0) {
    return <div className="py-8 text-center text-xs text-sf-muted">—</div>;
  }
  const lats = pts.map((p) => p[0]);
  const lons = pts.map((p) => p[1]);
  let minLat = Math.min(...lats), maxLat = Math.max(...lats);
  let minLon = Math.min(...lons), maxLon = Math.max(...lons);
  const padLat = Math.max(3, (maxLat - minLat) * 0.18);
  const padLon = Math.max(3, (maxLon - minLon) * 0.18);
  minLat -= padLat; maxLat += padLat; minLon -= padLon; maxLon += padLon;
  const px = (lon: number) => ((lon - minLon) / (maxLon - minLon || 1)) * W;
  const py = (lat: number) => ((maxLat - lat) / (maxLat - minLat || 1)) * H;

  const airports = new Map<string, [number, number]>();
  for (const f of flights) {
    if (f.originIcao && f.originLat != null) airports.set(f.originIcao, [f.originLat, f.originLon]);
    if (f.destinationIcao && f.destinationLat != null && f.destinationLon != null)
      airports.set(f.destinationIcao, [f.destinationLat, f.destinationLon]);
  }

  // (v6.1) Continentes de fondo (ne_110m_land) proyectados en equirectangular y
  // recortados al encuadre. Da un mapa real detrás de las rutas (app + PDF).
  const landPaths: string[] = [];
  for (const ft of (worldLand as { features: Array<{ geometry: { type: string; coordinates: number[][][] | number[][][][] } }> }).features) {
    const g = ft.geometry;
    const polys: number[][][][] =
      g.type === "Polygon"
        ? [g.coordinates as number[][][]]
        : g.type === "MultiPolygon"
          ? (g.coordinates as number[][][][])
          : [];
    for (const poly of polys) {
      const outer = poly[0];
      let mnx = 999, mxx = -999, mny = 999, mxy = -999;
      for (const c of outer) {
        if (c[0] < mnx) mnx = c[0];
        if (c[0] > mxx) mxx = c[0];
        if (c[1] < mny) mny = c[1];
        if (c[1] > mxy) mxy = c[1];
      }
      if (mxx < minLon || mnx > maxLon || mxy < minLat || mny > maxLat) continue;
      let d = "";
      for (const ring of poly) {
        d += "M" + ring.map((c) => `${px(c[0]).toFixed(1)} ${py(c[1]).toFixed(1)}`).join("L") + "Z";
      }
      landPaths.push(d);
    }
  }

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" style={{ display: "block" }}>
      <defs>
        <clipPath id="mapClip">
          <rect x="0" y="0" width={W} height={H} rx="8" />
        </clipPath>
      </defs>
      <rect x="0" y="0" width={W} height={H} rx="8" fill="#cfe2f3" />
      {/* Continentes */}
      <g clipPath="url(#mapClip)">
        {landPaths.map((d, i) => (
          <path key={i} d={d} fill="#e7ecd6" stroke="#bcc8a2" strokeWidth="0.6" fillRule="evenodd" />
        ))}
      </g>
      {/* Graticula */}
      {Array.from({ length: 7 }, (_, i) => {
        const x = (W / 6) * i;
        const y = (H / 6) * i;
        return (
          <g key={i} stroke="#bcd2f0" strokeWidth="0.6">
            <line x1={x} y1="0" x2={x} y2={H} />
            <line x1="0" y1={y} x2={W} y2={y} />
          </g>
        );
      })}
      {/* Rutas */}
      {flights.map((f, i) => {
        if (f.originLat == null || f.destinationLat == null || f.destinationLon == null) return null;
        const x1 = px(f.originLon), y1 = py(f.originLat);
        const x2 = px(f.destinationLon), y2 = py(f.destinationLat);
        const mx2 = (x1 + x2) / 2, my2 = (y1 + y2) / 2 - Math.abs(x2 - x1) * 0.12;
        return (
          <path key={i} d={`M${x1} ${y1} Q${mx2} ${my2} ${x2} ${y2}`} fill="none" stroke="#1d4ed8" strokeOpacity="0.55" strokeWidth="1.4" />
        );
      })}
      {/* Aeropuertos */}
      {[...airports.entries()].map(([icao, [lat, lon]], i) => (
        <g key={i}>
          <circle cx={px(lon)} cy={py(lat)} r="3.4" fill="#dc2626" stroke="#fff" strokeWidth="1" />
          <text x={px(lon) + 5} y={py(lat) + 3} fontSize="9" fill="#0f172a" fontWeight="600">{icao}</text>
        </g>
      ))}
    </svg>
  );
}

function AircraftReportModal({
  ac, tl, onClose,
}: {
  ac: HangarAircraft; tl: TechLogAircraft | null; onClose: () => void;
}) {
  const entries = useFlightLogStore((s) => s.entries);
  const push = useToastStore((s) => s.push);
  const photo = useAircraftPhoto(ac.registration ?? null);
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : "—";
  const reg = ac.registration?.toUpperCase() ?? null;
  const flights = useMemo(
    () =>
      entries.filter((e) =>
        reg
          ? e.aircraftRegistration?.toUpperCase() === reg
          : (e.aircraftModel ?? "").toUpperCase() === (ac.model ?? "").toUpperCase(),
      ),
    [entries, reg, ac.model],
  );
  const recent = flights.slice(0, 12);

  const doPrint = () => {
    const onAfter = () => {
      push({
        kind: "success",
        title: t("hangar.report.saved"),
        message: t("hangar.report.saved_hint"),
        ttlMs: 6000,
      });
      window.removeEventListener("afterprint", onAfter);
    };
    window.addEventListener("afterprint", onAfter);
    window.print();
  };

  return (
    <div className="hangar-print-overlay fixed inset-0 z-[120] flex items-start justify-center overflow-y-auto bg-slate-950/80 p-6 backdrop-blur-sm" onClick={onClose}>
      <div
        onClick={(e) => e.stopPropagation()}
        className="hangar-print-area w-full max-w-3xl rounded-2xl bg-white p-8 text-slate-900 shadow-2xl"
      >
        {/* Encabezado */}
        <div className="flex items-center justify-between border-b-2 border-sf-border pb-4">
          <div className="flex items-center gap-3">
            <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={40} />
            <div>
              <div className="text-xl font-bold">{ac.registration ?? model}</div>
              <div className="text-xs text-sf-muted">
                {[ac.airlineName, model].filter(Boolean).join(" · ")}
              </div>
            </div>
          </div>
          <div className="text-right text-xs text-sf-muted">
            <div className="font-semibold text-sf-muted/70">{t("hangar.report.title")}</div>
            <div>{fmtDate(new Date().toISOString())}</div>
          </div>
        </div>

        {photo && (
          <img src={photo} alt="" className="mt-4 h-40 w-full rounded-lg object-cover" />
        )}

        {/* KPIs */}
        <div className="mt-4 grid grid-cols-3 gap-3 sm:grid-cols-6">
          <RKpi k={t("hangar.kpi.miles")} v={`${Math.round(ac.totalNm).toLocaleString()} nm`} />
          <RKpi k={t("hangar.mx.total_hours")} v={tl ? `${Math.round(tl.flightHours).toLocaleString()} h` : "—"} />
          <RKpi k={t("hangar.perf.cycles")} v={tl ? Math.round(tl.cycles).toLocaleString() : "—"} />
          <RKpi k={t("hangar.flights_short")} v={String(flights.length)} />
          <RKpi k={t("hangar.fpm.avg")} v={ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)}` : "—"} />
          <RKpi k={t("hangar.health_general")} v={tl ? `${Math.round(tl.condition.avgRemainingPct)}%` : "—"} />
        </div>

        {/* Mapa de rutas */}
        <div className="mt-5">
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-sf-muted">{t("hangar.report.routes")}</h3>
          <FlightMapSvg flights={flights} />
        </div>

        {/* (20/10) Mantenimiento: estado real del Tech Log. */}
        {tl && (
          <div className="mt-5">
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-sf-muted">
              {t("hangar.report.maintenance", { app: tl.appName })}
            </h3>
            <div className="mb-2 flex flex-wrap gap-x-6 gap-y-1 text-sm">
              <span>
                {t("hangar.report.dispatch")}:{" "}
                <span className="font-semibold">
                  {tl.dispatch.status === "NO_GO" ? "NO-GO" : tl.dispatch.status === "GO_MEL" ? "GO · MEL" : "GO"}
                </span>
              </span>
              <span>
                {t("techlog.summary.open")}: <span className="font-semibold">{tl.defects.filter((d) => d.status === "Open").length}</span>
              </span>
              <span>
                {t("techlog.summary.deferred")}: <span className="font-semibold">{tl.defects.filter((d) => d.status === "Deferred").length}</span>
              </span>
              <span>
                {t("techlog.parts.condition", { good: tl.condition.good, low: tl.condition.low, failed: tl.condition.failed })}
              </span>
            </div>
            {tl.parts.length > 0 && (
              <div className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
                {tl.parts.slice(0, 9).map((p) => (
                  <div key={p.id} className="flex justify-between gap-2 border-b border-slate-200 py-0.5">
                    <span className="truncate text-sf-muted/70">{p.name}</span>
                    <span className="font-semibold">{Math.round(p.remainingPct)}%</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Vuelos recientes */}
        <div className="mt-5">
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-sf-muted">{t("hangar.report.recent")}</h3>
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-slate-300 text-xs uppercase tracking-wide text-sf-muted">
                <th className="py-1">{t("hangar.flights.date")}</th>
                <th className="py-1">{t("hangar.flights.dest")}</th>
                <th className="py-1 text-right">{t("hangar.flights.time")}</th>
                <th className="py-1 text-right">FPM</th>
              </tr>
            </thead>
            <tbody>
              {recent.map((f) => (
                <tr key={f.id} className="border-b border-slate-200">
                  <td className="py-1">{fmtDate(f.startedAt)}</td>
                  <td className="py-1">{f.originIcao ?? "—"} → {f.destinationIcao ?? "—"}</td>
                  <td className="py-1 text-right">{f.flightTimeS ? hours(f.flightTimeS) : "—"}</td>
                  <td className="py-1 text-right">{f.landingFpm ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <p className="mt-6 text-[10px] text-sf-muted">{t("hangar.report.disclaimer")}</p>

        <div className="mt-6 flex justify-end gap-2 print:hidden">
          <button onClick={onClose} className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-sm text-sf-muted/70 hover:bg-slate-100">
            <X className="h-4 w-4" /> {t("common.close")}
          </button>
          <button onClick={doPrint} className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-slate-950 hover:bg-brand-400">
            <Printer className="h-4 w-4" /> {t("hangar.docs.print")}
          </button>
        </div>
      </div>
    </div>
  );
}

function RKpi({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-lg bg-slate-100 p-2 text-center">
      <div className="text-base font-bold text-slate-900">{v}</div>
      <div className="text-[10px] uppercase tracking-wide text-sf-muted">{k}</div>
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
    <div className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <h3 className="mb-2 flex items-center gap-1.5 text-base font-semibold text-sf-text">
        <Gauge className="h-4 w-4 text-brand-400" />
        {t("hangar.fpm.title", { n: String(n) })}
        <Info className="h-3 w-3 text-sf-muted/70" />
      </h3>
      {n === 0 ? (
        <div className="flex h-40 items-center justify-center text-sm text-sf-muted/70">
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
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-sf-border pt-3">
        <div className="flex flex-wrap gap-3 text-[12px]">
          <Legend color={GRADE_COLOR.butter} label={t("hangar.fpm.butter")} n={counts.butter ?? 0} />
          <Legend color={GRADE_COLOR.acceptable} label={t("hangar.fpm.acceptable")} n={counts.acceptable ?? 0} />
          <Legend color={GRADE_COLOR.hard} label={t("hangar.fpm.hard")} n={counts.hard ?? 0} />
        </div>
        <div className="text-right">
          <div className="text-[10px] uppercase tracking-wide text-sf-muted">
            {t("hangar.fpm.avg")}
          </div>
          <div className="text-base font-semibold text-sf-text">{avg}</div>
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
    <span className="inline-flex items-center gap-1 text-sf-muted">
      <span className="h-2 w-2 rounded-full" style={{ backgroundColor: color }} />
      {label} <span className="font-semibold text-sf-text">{n}</span>
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
        <h3 className="flex items-center gap-1.5 text-sm font-semibold text-sf-text">
          <Film className="h-4 w-4 text-brand-400" />
          {t("hangar.best_landings.title")}
          <Info className="h-3 w-3 text-sf-muted/70" />
        </h3>
        {hasClips && (
          <span className="text-[11px] text-sf-muted">
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
          <p className="mt-1 text-[11px] text-sf-muted/70">
            {t("hangar.best_landings.enable_hint")}
          </p>
        </>
      ) : (
        <div className="flex flex-col items-center justify-center gap-1 rounded-xl border border-dashed border-sf-border bg-sf-surface/60 py-8 text-sf-muted">
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
    <div className="group w-full overflow-hidden rounded-xl border border-sf-border bg-sf-bg/40">
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
          <span className="font-mono text-2xl font-bold text-sf-text/90/70">
            {clip.airportIcao ?? "✈"}
          </span>
        )}
        <span className="absolute inset-0 flex items-center justify-center bg-slate-950/0 transition-colors group-hover:bg-sf-bg/40">
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
          <span className="absolute left-2 top-2 rounded bg-slate-950/70 px-1.5 py-0.5 text-[10px] text-sf-muted">
            {t("hangar.clip.test")}
          </span>
        )}
      </button>
      <div className="flex items-center justify-between gap-1 p-2.5">
        <div className="min-w-0">
          <div className="truncate text-xs font-medium text-sf-text">
            {clip.airportName ?? clip.airportIcao ?? t("hangar.clip.test")}
          </div>
          <div className="truncate text-[10px] text-sf-muted">
            {fmtDate(clip.recordedAt)}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <button
            onClick={toggleFav}
            title={t("hangar.clip.favorite")}
            className={`rounded p-1 hover:bg-sf-raised ${
              clip.favorite ? "text-amber-400" : "text-sf-muted"
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
            className="rounded p-1 text-sf-muted hover:bg-sf-raised hover:text-rose-400"
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
    <div className="w-full overflow-hidden rounded-xl border border-sf-border bg-sf-bg/40">
      <div className="relative flex aspect-video items-center justify-center bg-gradient-to-br from-slate-700/50 via-slate-900 to-slate-950">
        <span className="font-mono text-2xl font-bold text-sf-text/90/80">
          {l.icao ?? "—"}
        </span>
        <span
          className="absolute right-2 top-2 rounded-md px-1.5 py-0.5 text-[10px] font-semibold text-slate-950"
          style={{ backgroundColor: color }}
        >
          {l.fpm} fpm · {gradeLabel}
        </span>
      </div>
      <div className="p-2.5">
        <div className="truncate text-xs font-medium text-sf-text">
          {l.airportName ?? l.icao ?? "—"}
        </div>
        <div className="mt-0.5 flex items-center justify-between text-[10px] text-sf-muted">
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
