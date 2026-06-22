import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
  Tooltip,
} from "recharts";
import {
  ArrowLeft,
  TrendingUp,
  TrendingDown,
  Plane,
  Users,
  Smile,
  Wallet,
  Landmark,
  Wrench,
  ConciergeBell,
  X,
  Coffee,
  Utensils,
  Wifi,
  Armchair,
  Luggage,
  CheckCircle2,
} from "lucide-react";
import type { AirlineLedger, AirlinePolicy } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useAppStore } from "../stores/useAppStore";
import { AirlineLogo } from "./AirlineLogo";
import { MaintenanceModal } from "./AircraftMaintenance";

// ───────────────────────────── helpers de formato ────────────────────────────

const usdCompact = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  notation: "compact",
  maximumFractionDigits: 1,
});
const usdFull = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumFractionDigits: 0,
});
const intFmt = new Intl.NumberFormat("en-US");

/** Compacto con signo para resultados (ej. +$3.2M, -$420.0K). */
function fmtSigned(n: number): string {
  const s = usdCompact.format(Math.abs(n));
  return n < 0 ? `−${s}` : `+${s}`;
}

/** Color + etiqueta de satisfacción (0-100). La marca el FPM de aterrizaje. */
function satTier(sat: number): { color: string; label: string } {
  if (sat >= 90) return { color: "#22c55e", label: t("economy.sat.excellent") };
  if (sat >= 75) return { color: "#84cc16", label: t("economy.sat.good") };
  if (sat >= 55) return { color: "#f59e0b", label: t("economy.sat.fair") };
  return { color: "#ef4444", label: t("economy.sat.poor") };
}

/** Tipo de aerolínea para el badge (carga / low-cost / tradicional). */
function airlineKind(led: AirlineLedger): { label: string; color: string } {
  if (led.cargo) return { label: t("economy.badge.cargo"), color: "#fb923c" };
  if (led.lowcost) return { label: t("economy.badge.lowcost"), color: "#fbbf24" };
  return { label: t("economy.badge.legacy"), color: "#38bdf8" };
}

const REV_COLORS = { tickets: "#0ea5e9", ancillary: "#22c55e" };
const COST_COLORS = {
  fuel: "#ef4444",
  catering: "#f59e0b",
  handling: "#a855f7",
  maintenance: "#14b8a6",
  fees: "#64748b",
};

type SortKey = "net" | "balance" | "satisfaction";

// ──────────────────────────────── vista raíz ─────────────────────────────────

export function EconomyView() {
  const focus = useAppStore((s) => s.economyFocus);
  const openEconomy = useAppStore((s) => s.openEconomy);
  const maintReg = useAppStore((s) => s.economyMaintReg);
  const clearMaintReg = useAppStore((s) => s.clearEconomyMaintReg);

  const [list, setList] = useState<AirlineLedger[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setList(await api.airlineEconomy());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const focused = useMemo(() => {
    if (!focus || !list) return null;
    const f = focus.toUpperCase();
    return (
      list.find(
        (l) => l.key.toUpperCase() === f || (l.icao && l.icao.toUpperCase() === f),
      ) ?? null
    );
  }, [focus, list]);

  return (
    <div className="h-full overflow-y-auto pr-1">
      <header className="mb-4">
        <h1 className="flex items-center gap-2 text-xl font-semibold text-slate-100">
          <Wallet className="h-5 w-5 text-brand-400" />
          {t("economy.title")}
        </h1>
        <p className="mt-1 text-sm text-slate-500">{t("economy.subtitle")}</p>
      </header>

      {error && (
        <div className="rounded-xl border border-red-900/50 bg-red-950/30 p-4 text-sm text-red-300">
          {t("economy.error")}: {error}
        </div>
      )}

      {!error && !list && (
        <div className="py-20 text-center text-sm text-slate-500">
          {t("economy.loading")}
        </div>
      )}

      {!error && list && list.length === 0 && (
        <div className="rounded-2xl border border-slate-800 bg-slate-900/40 p-10 text-center">
          <p className="text-base font-semibold text-slate-200">
            {t("economy.empty.title")}
          </p>
          <p className="mt-1 text-sm text-slate-500">{t("economy.empty.hint")}</p>
        </div>
      )}

      {!error && list && list.length > 0 && focused && (
        <AirlineDetail
          led={focused}
          onBack={() => openEconomy(null)}
          onChanged={load}
          initialMaintReg={maintReg}
          onConsumeMaint={clearMaintReg}
        />
      )}

      {!error && list && list.length > 0 && !focused && (
        <Overview
          list={list}
          notFound={!!focus}
          onSelect={(led) => openEconomy(led.icao ?? led.key)}
        />
      )}
    </div>
  );
}

// ──────────────────────────────── overview ───────────────────────────────────

function Overview({
  list,
  notFound,
  onSelect,
}: {
  list: AirlineLedger[];
  notFound: boolean;
  onSelect: (led: AirlineLedger) => void;
}) {
  const [sort, setSort] = useState<SortKey>("net");

  const sorted = useMemo(() => {
    const arr = [...list];
    arr.sort((a, b) => {
      if (sort === "balance") return b.balance - a.balance;
      if (sort === "satisfaction")
        return (b.satisfaction ?? -1) - (a.satisfaction ?? -1);
      return b.net - a.net;
    });
    return arr;
  }, [list, sort]);

  return (
    <>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <span className="text-xs text-slate-500">
          {t("economy.overview.count", { n: list.length })}
        </span>
        <span className="ml-auto text-[11px] text-slate-600">
          {t("economy.click_hint")}
        </span>
        <div className="flex items-center gap-1 rounded-lg bg-slate-900/70 p-0.5 ring-1 ring-slate-800">
          {(["net", "balance", "satisfaction"] as SortKey[]).map((k) => (
            <button
              key={k}
              onClick={() => setSort(k)}
              className={`rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors ${
                sort === k
                  ? "bg-brand-500/20 text-brand-200"
                  : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {t(`economy.sort.${k}`)}
            </button>
          ))}
        </div>
      </div>

      {notFound && (
        <div className="mb-3 rounded-lg border border-amber-900/40 bg-amber-950/20 px-3 py-2 text-xs text-amber-300/90">
          {t("economy.not_found")}
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {sorted.map((led) => (
          <AirlineCard key={led.key} led={led} onClick={() => onSelect(led)} />
        ))}
      </div>
    </>
  );
}

function AirlineCard({ led, onClick }: { led: AirlineLedger; onClick: () => void }) {
  const profit = led.net >= 0;
  const margin = led.revenue > 0 ? (led.net / led.revenue) * 100 : 0;
  const sat = led.satisfaction;
  const tier = sat != null ? satTier(sat) : null;

  return (
    <button
      onClick={onClick}
      className="group flex flex-col gap-3 rounded-2xl border border-slate-800 bg-slate-900/40 p-4 text-left transition-colors hover:border-brand-500/50 hover:bg-slate-900/70"
    >
      <div className="flex items-center gap-3">
        <AirlineLogo icao={led.icao} name={led.name} size={40} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-slate-100">
            {led.name}
          </div>
          <span
            className="text-[10px] font-medium"
            style={{ color: airlineKind(led).color }}
          >
            {airlineKind(led).label}
          </span>
        </div>
        <span
          className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold ${
            profit
              ? "bg-emerald-500/15 text-emerald-300"
              : "bg-red-500/15 text-red-300"
          }`}
        >
          {profit ? (
            <TrendingUp className="h-3 w-3" />
          ) : (
            <TrendingDown className="h-3 w-3" />
          )}
          {profit ? t("economy.market.profit") : t("economy.market.loss")}
        </span>
      </div>

      <div className="flex items-end justify-between">
        <div>
          <div className="text-[10px] uppercase tracking-wide text-slate-500">
            {t("economy.kpi.balance")}
          </div>
          <div className="text-lg font-bold text-slate-100">
            {usdCompact.format(led.balance)}
          </div>
        </div>
        <div className="text-right">
          <div className="text-[10px] uppercase tracking-wide text-slate-500">
            {t("economy.kpi.net")}
          </div>
          <div
            className={`text-sm font-semibold ${
              profit ? "text-emerald-400" : "text-red-400"
            }`}
          >
            {fmtSigned(led.net)}
          </div>
        </div>
      </div>

      {/* Satisfacción */}
      <div>
        <div className="mb-1 flex items-center justify-between text-[10px]">
          <span className="flex items-center gap-1 text-slate-500">
            <Smile className="h-3 w-3" />
            {t("economy.kpi.satisfaction")}
          </span>
          {tier ? (
            <span style={{ color: tier.color }} className="font-semibold">
              {Math.round(sat!)}% · {tier.label}
            </span>
          ) : (
            <span className="text-slate-600">{t("economy.sat.nodata")}</span>
          )}
        </div>
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
          <div
            className="h-full rounded-full"
            style={{
              width: `${sat ?? 0}%`,
              background: tier?.color ?? "#475569",
            }}
          />
        </div>
      </div>

      <div className="flex items-center gap-3 text-[11px] text-slate-500">
        <span className="flex items-center gap-1">
          <Plane className="h-3 w-3" /> {intFmt.format(led.flights)}
        </span>
        <span className="flex items-center gap-1">
          <Users className="h-3 w-3" /> {intFmt.format(led.passengers)}
        </span>
        <span className="ml-auto font-medium text-slate-400">
          {margin >= 0 ? "+" : ""}
          {margin.toFixed(0)}% {t("economy.kpi.margin").toLowerCase()}
        </span>
      </div>
    </button>
  );
}

// ──────────────────────────────── detalle ────────────────────────────────────

function AirlineDetail({
  led,
  onBack,
  onChanged,
  initialMaintReg,
  onConsumeMaint,
}: {
  led: AirlineLedger;
  onBack: () => void;
  onChanged: () => void | Promise<void>;
  initialMaintReg?: string | null;
  onConsumeMaint?: () => void;
}) {
  const [modal, setModal] = useState<"maintenance" | "services" | null>(null);
  const [maintReg, setMaintReg] = useState<string | null>(null);

  // Si venimos del Hangar con una matrícula, abre directo el mantenimiento.
  useEffect(() => {
    if (initialMaintReg) {
      setMaintReg(initialMaintReg);
      setModal("maintenance");
      onConsumeMaint?.();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialMaintReg]);
  const profit = led.net >= 0;
  const margin = led.revenue > 0 ? (led.net / led.revenue) * 100 : 0;
  const sat = led.satisfaction;
  const tier = sat != null ? satTier(sat) : null;

  const revData = [
    {
      name: led.cargo ? t("economy.revenue.freight") : t("economy.revenue.tickets"),
      value: led.revenueTickets,
      color: REV_COLORS.tickets,
    },
    { name: t("economy.revenue.ancillary"), value: led.revenueAncillary, color: REV_COLORS.ancillary },
  ].filter((d) => d.value > 0);

  const costData = [
    { name: t("economy.costs.fuel"), value: led.costFuel, color: COST_COLORS.fuel },
    { name: t("economy.costs.catering"), value: led.costCatering, color: COST_COLORS.catering },
    { name: t("economy.costs.handling"), value: led.costHandling, color: COST_COLORS.handling },
    { name: t("economy.costs.maintenance"), value: led.costMaintenance, color: COST_COLORS.maintenance },
    { name: t("economy.costs.fees"), value: led.costFees, color: COST_COLORS.fees },
  ].filter((d) => d.value > 0);

  return (
    <div className="space-y-4">
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs font-medium text-slate-400 hover:text-slate-200"
      >
        <ArrowLeft className="h-3.5 w-3.5" />
        {t("economy.back")}
      </button>

      {/* Cabecera */}
      <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-slate-800 bg-slate-900/40 p-4">
        <AirlineLogo icao={led.icao} name={led.name} size={56} />
        <div className="min-w-0">
          <h2 className="text-lg font-bold text-slate-100">{led.name}</h2>
          <div className="mt-0.5 flex items-center gap-2 text-[11px]">
            <span
              className="rounded-full px-2 py-0.5 font-medium"
              style={{
                color: airlineKind(led).color,
                background: `${airlineKind(led).color}22`,
              }}
            >
              {airlineKind(led).label}
            </span>
            {led.icao && <span className="text-slate-500">{led.icao}</span>}
          </div>
        </div>
        <span
          className={`ml-auto flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-semibold ${
            profit
              ? "bg-emerald-500/15 text-emerald-300"
              : "bg-red-500/15 text-red-300"
          }`}
        >
          {profit ? <TrendingUp className="h-4 w-4" /> : <TrendingDown className="h-4 w-4" />}
          {profit ? t("economy.market.profit") : t("economy.market.loss")}
        </span>
      </div>

      {/* (v6.1) Gestión de la aerolínea — el ecosistema */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <button
          onClick={() => setModal("maintenance")}
          className="flex items-center gap-3 rounded-2xl border border-slate-800 bg-slate-900/40 p-4 text-left transition-colors hover:border-brand-500/50 hover:bg-slate-900/70"
        >
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-amber-500/15 text-amber-300">
            <Wrench className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <div className="text-sm font-semibold text-slate-100">
              {t("economy.manage.maintenance")}
            </div>
            <div className="text-xs text-slate-500">
              {t("economy.manage.maintenance.hint")}
            </div>
          </div>
        </button>
        {!led.cargo && (
          <button
            onClick={() => setModal("services")}
            className="flex items-center gap-3 rounded-2xl border border-slate-800 bg-slate-900/40 p-4 text-left transition-colors hover:border-brand-500/50 hover:bg-slate-900/70"
          >
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sky-500/15 text-sky-300">
              <ConciergeBell className="h-5 w-5" />
            </span>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-slate-100">
                {t("economy.manage.services")}
              </div>
              <div className="text-xs text-slate-500">
                {t("economy.manage.services.hint")}
              </div>
            </div>
          </button>
        )}
      </div>

      {/* KPIs */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-7">
        <Kpi
          icon={<Landmark className="h-4 w-4" />}
          label={t("economy.kpi.valuation")}
          value={usdCompact.format(led.valuation)}
          hint={t("economy.kpi.valuation.hint")}
        />
        <Kpi
          icon={<Wallet className="h-4 w-4" />}
          label={t("economy.kpi.balance")}
          value={usdCompact.format(led.balance)}
        />
        <Kpi
          label={t("economy.kpi.net")}
          value={fmtSigned(led.net)}
          valueClass={profit ? "text-emerald-400" : "text-red-400"}
        />
        <Kpi
          label={t("economy.kpi.margin")}
          value={`${margin >= 0 ? "+" : ""}${margin.toFixed(1)}%`}
          valueClass={profit ? "text-emerald-400" : "text-red-400"}
        />
        <Kpi
          icon={<Plane className="h-4 w-4" />}
          label={t("economy.kpi.flights")}
          value={intFmt.format(led.flights)}
        />
        <Kpi
          icon={<Users className="h-4 w-4" />}
          label={t("economy.kpi.passengers")}
          value={intFmt.format(led.passengers)}
        />
        <Kpi
          icon={<Plane className="h-4 w-4" />}
          label={t("economy.kpi.fleet")}
          value={usdCompact.format(led.fleetValue)}
          hint={t("economy.kpi.fleet.hint", { n: String(led.fleetSize) })}
        />
      </div>

      {/* Satisfacción del pasajero */}
      <div className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4">
        <h3 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-slate-200">
          <Smile className="h-4 w-4 text-brand-400" />
          {t("economy.kpi.satisfaction")}
        </h3>
        {tier ? (
          <>
            <div className="flex items-baseline gap-2">
              <span className="text-3xl font-bold" style={{ color: tier.color }}>
                {Math.round(sat!)}%
              </span>
              <span className="text-sm font-medium" style={{ color: tier.color }}>
                {tier.label}
              </span>
              {led.avgLandingFpm != null && (
                <span className="ml-auto text-xs text-slate-500">
                  {t("economy.satisfaction.avgfpm", {
                    fpm: Math.round(led.avgLandingFpm),
                  })}
                </span>
              )}
            </div>
            <div className="mt-2 h-2.5 w-full overflow-hidden rounded-full bg-slate-800">
              <div
                className="h-full rounded-full transition-all"
                style={{ width: `${sat}%`, background: tier.color }}
              />
            </div>
            <p className="mt-2 text-xs text-slate-500">
              {t("economy.satisfaction.hint")}
            </p>
          </>
        ) : (
          <p className="py-6 text-center text-sm text-slate-500">
            {t("economy.satisfaction.nodata")}
          </p>
        )}
      </div>

      {/* Ingresos vs Costes */}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <BreakdownCard
          title={t("economy.revenue.heading")}
          total={led.revenue}
          totalClass="text-emerald-400"
          data={revData}
        />
        <BreakdownCard
          title={t("economy.costs.heading")}
          total={led.costs}
          totalClass="text-red-400"
          data={costData}
        />
      </div>

      {modal === "maintenance" && (
        <MaintenanceModal
          led={led}
          initialReg={maintReg}
          onClose={() => {
            setModal(null);
            setMaintReg(null);
          }}
          onChanged={onChanged}
        />
      )}
      {modal === "services" && (
        <PolicyModal
          led={led}
          section="services"
          onClose={() => setModal(null)}
          onSaved={async () => {
            setModal(null);
            await onChanged();
          }}
        />
      )}
    </div>
  );
}

function Kpi({
  icon,
  label,
  value,
  hint,
  valueClass = "text-slate-100",
}: {
  icon?: React.ReactNode;
  label: string;
  value: string;
  hint?: string;
  valueClass?: string;
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      <div className="flex items-center gap-1 text-[10px] uppercase tracking-wide text-slate-500">
        {icon}
        {label}
      </div>
      <div className={`mt-1 text-lg font-bold ${valueClass}`}>{value}</div>
      {hint && <div className="text-[10px] text-slate-600">{hint}</div>}
    </div>
  );
}

function BreakdownCard({
  title,
  total,
  totalClass,
  data,
}: {
  title: string;
  total: number;
  totalClass: string;
  data: Array<{ name: string; value: number; color: string }>;
}) {
  return (
    <div className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4">
      <div className="mb-2 flex items-baseline justify-between">
        <h3 className="text-sm font-semibold text-slate-200">{title}</h3>
        <span className={`text-base font-bold ${totalClass}`}>
          {usdFull.format(total)}
        </span>
      </div>
      {data.length === 0 ? (
        <p className="py-10 text-center text-xs text-slate-600">
          {t("economy.no_data")}
        </p>
      ) : (
        <div className="flex items-center gap-3">
          <div className="h-36 w-36 shrink-0">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={data}
                  dataKey="value"
                  nameKey="name"
                  innerRadius={38}
                  outerRadius={64}
                  paddingAngle={2}
                  stroke="none"
                  isAnimationActive={false}
                >
                  {data.map((d) => (
                    <Cell key={d.name} fill={d.color} />
                  ))}
                </Pie>
                <Tooltip
                  formatter={(v) => usdFull.format(Number(v))}
                  contentStyle={{
                    background: "#020617",
                    border: "1px solid #334155",
                    borderRadius: 8,
                    fontSize: 12,
                  }}
                />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <ul className="min-w-0 flex-1 space-y-1.5">
            {data.map((d) => {
              const pct = total > 0 ? (d.value / total) * 100 : 0;
              return (
                <li key={d.name} className="flex items-center gap-2 text-xs">
                  <span
                    className="h-2.5 w-2.5 shrink-0 rounded-sm"
                    style={{ background: d.color }}
                  />
                  <span className="min-w-0 flex-1 truncate text-slate-400">
                    {d.name}
                  </span>
                  <span className="font-medium text-slate-300">
                    {usdCompact.format(d.value)}
                  </span>
                  <span className="w-9 text-right text-slate-600">
                    {pct.toFixed(0)}%
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

// ──────────────────────── Modal de gestión (ecosistema) ──────────────────────

type ServiceKey =
  | "snacks"
  | "meals"
  | "wifi"
  | "seatUpgrades"
  | "priorityBoarding"
  | "extraBaggage";

const SERVICES: Array<{
  key: ServiceKey;
  icon: React.ReactNode;
  labelKey: string;
  perPax: string;
}> = [
  { key: "snacks", icon: <Coffee className="h-4 w-4" />, labelKey: "economy.svc.snacks", perPax: "+$3" },
  { key: "meals", icon: <Utensils className="h-4 w-4" />, labelKey: "economy.svc.meals", perPax: "+$4" },
  { key: "wifi", icon: <Wifi className="h-4 w-4" />, labelKey: "economy.svc.wifi", perPax: "+$2.5" },
  { key: "seatUpgrades", icon: <Armchair className="h-4 w-4" />, labelKey: "economy.svc.seat", perPax: "+$3" },
  { key: "priorityBoarding", icon: <CheckCircle2 className="h-4 w-4" />, labelKey: "economy.svc.priority", perPax: "+$2" },
  { key: "extraBaggage", icon: <Luggage className="h-4 w-4" />, labelKey: "economy.svc.baggage", perPax: "+$5" },
];

const MAINT_LEVELS = [0, 1, 2] as const;

function PolicyModal({
  led,
  section,
  onClose,
  onSaved,
}: {
  led: AirlineLedger;
  section: "maintenance" | "services";
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const [policy, setPolicy] = useState<AirlinePolicy | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    api
      .airlinePolicy(led.key)
      .then((p) => alive && setPolicy(p))
      .catch(() => alive && setPolicy(null));
    return () => {
      alive = false;
    };
  }, [led.key]);

  const save = async () => {
    if (!policy) return;
    setSaving(true);
    try {
      await api.setAirlinePolicy(led.key, policy);
      await onSaved();
    } finally {
      setSaving(false);
    }
  };

  const title =
    section === "maintenance"
      ? t("economy.modal.maintenance.title")
      : t("economy.modal.services.title");
  const desc =
    section === "maintenance"
      ? t("economy.modal.maintenance.desc")
      : t("economy.modal.services.desc");

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onClose}
    >
      <div
        className="max-h-[85vh] w-full max-w-md overflow-y-auto rounded-2xl border border-slate-700 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 border-b border-slate-800 p-4">
          <AirlineLogo icao={led.icao} name={led.name} size={32} />
          <div className="min-w-0 flex-1">
            <h3 className="truncate text-sm font-semibold text-slate-100">{title}</h3>
            <p className="truncate text-xs text-slate-500">{led.name}</p>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-300"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="p-4">
          <p className="mb-3 text-xs text-slate-500">{desc}</p>

          {!policy ? (
            <p className="py-8 text-center text-sm text-slate-500">
              {t("economy.modal.loading")}
            </p>
          ) : section === "maintenance" ? (
            <div className="space-y-2">
              {MAINT_LEVELS.map((lvl) => {
                const active = policy.maintenanceLevel === lvl;
                const name = ["basic", "standard", "premium"][lvl];
                return (
                  <button
                    key={lvl}
                    onClick={() => setPolicy({ ...policy, maintenanceLevel: lvl })}
                    className={`flex w-full items-start gap-3 rounded-xl border p-3 text-left transition-colors ${
                      active
                        ? "border-brand-500/60 bg-brand-500/10"
                        : "border-slate-800 bg-slate-950/40 hover:bg-slate-800/40"
                    }`}
                  >
                    <span
                      className={`mt-0.5 h-3.5 w-3.5 shrink-0 rounded-full border-2 ${
                        active ? "border-brand-400 bg-brand-400" : "border-slate-600"
                      }`}
                    />
                    <div>
                      <div className="text-sm font-semibold text-slate-100">
                        {t(`economy.maint.${name}`)}
                      </div>
                      <div className="text-xs text-slate-500">
                        {t(`economy.maint.${name}.desc`)}
                      </div>
                    </div>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="space-y-2">
              {SERVICES.map((s) => {
                const on = policy[s.key];
                return (
                  <button
                    key={s.key}
                    onClick={() => setPolicy({ ...policy, [s.key]: !on })}
                    className="flex w-full items-center gap-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-left transition-colors hover:bg-slate-800/40"
                  >
                    <span
                      className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${
                        on ? "bg-sky-500/20 text-sky-300" : "bg-slate-800 text-slate-500"
                      }`}
                    >
                      {s.icon}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="text-sm font-medium text-slate-100">
                        {t(s.labelKey)}
                      </div>
                      <div className="text-[11px] text-emerald-400/80">
                        {s.perPax}/pax
                      </div>
                    </div>
                    <span
                      className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
                        on ? "bg-brand-500" : "bg-slate-700"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all ${
                          on ? "left-[18px]" : "left-0.5"
                        }`}
                      />
                    </span>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-slate-800 p-4">
          <button
            onClick={onClose}
            className="rounded-lg px-3 py-1.5 text-sm font-medium text-slate-400 hover:text-slate-200"
          >
            {t("economy.modal.cancel")}
          </button>
          <button
            onClick={save}
            disabled={!policy || saving}
            className="rounded-lg bg-brand-500 px-4 py-1.5 text-sm font-semibold text-white hover:bg-brand-400 disabled:opacity-50"
          >
            {saving ? t("economy.modal.saving") : t("economy.modal.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
