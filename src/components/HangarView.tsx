import { useEffect, useState } from "react";
import {
  Plane,
  MapPin,
  Route,
  Clock,
  Gauge,
  AlertTriangle,
  ShieldCheck,
  Wrench,
  Film,
} from "lucide-react";
import type { HangarAnalytics, HangarAircraft } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { AirlineLogo } from "./AirlineLogo";
import { cleanAtcType } from "../lib/aircraft";

/**
 * (v6 #2a) Vista "Hangar & Analytics".
 *
 * Telemetría agregada del FlightBook: top aviones por matrícula (modelo +
 * logo/aerolínea + registro + millas + tiempo), aeropuertos más frecuentados
 * y "salud" de cada avión derivada del FPM de aterrizaje (mantenimiento/
 * desgaste). El módulo "Best Landings" (grabación nativa) llega en #2b.
 */
export function HangarView() {
  const [data, setData] = useState<HangarAnalytics | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .hangarAnalytics()
      .then((d) => {
        if (!cancelled) setData(d);
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
    <div className="h-full overflow-y-auto pb-6">
      {/* KPIs */}
      <div className="grid grid-cols-3 gap-3">
        <Kpi
          icon={<Plane className="h-4 w-4" />}
          label={t("hangar.kpi.flights")}
          value={data.totalFlights.toLocaleString()}
        />
        <Kpi
          icon={<Route className="h-4 w-4" />}
          label={t("hangar.kpi.miles")}
          value={`${Math.round(data.totalNm).toLocaleString()} nm`}
        />
        <Kpi
          icon={<Clock className="h-4 w-4" />}
          label={t("hangar.kpi.hours")}
          value={fmtHours(data.totalTimeS)}
        />
      </div>

      {/* Top aviones */}
      <h2 className="mt-6 mb-2 flex items-center gap-2 text-sm font-semibold text-slate-200">
        <Plane className="h-4 w-4 text-brand-400" />
        {t("hangar.top_aircraft")}
      </h2>
      <div className="space-y-2">
        {data.aircraft.map((ac) => (
          <AircraftRow key={ac.key} ac={ac} />
        ))}
      </div>

      {/* Aeropuertos más frecuentados */}
      <h2 className="mt-6 mb-2 flex items-center gap-2 text-sm font-semibold text-slate-200">
        <MapPin className="h-4 w-4 text-brand-400" />
        {t("hangar.top_airports")}
      </h2>
      <div className="flex flex-wrap gap-2">
        {data.airports.map((ap) => (
          <div
            key={ap.icao}
            className="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-1.5"
            title={ap.name ?? undefined}
          >
            <span className="font-mono text-xs font-semibold text-slate-200">
              {ap.icao}
            </span>
            <span className="text-[11px] text-slate-500">×{ap.visits}</span>
          </div>
        ))}
      </div>

      {/* Best Landings — placeholder de #2b */}
      <div className="mt-6 flex items-center gap-3 rounded-xl border border-dashed border-slate-700 bg-slate-900/40 p-4 text-slate-400">
        <Film className="h-5 w-5 shrink-0 text-slate-500" />
        <div>
          <p className="text-sm font-medium text-slate-300">
            {t("hangar.best_landings.title")}
          </p>
          <p className="text-[11px] text-slate-500">
            {t("hangar.best_landings.soon")}
          </p>
        </div>
      </div>
    </div>
  );
}

function Kpi({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3">
      <div className="flex items-center gap-1.5 text-slate-500">
        {icon}
        <span className="text-[11px] uppercase tracking-wide">{label}</span>
      </div>
      <div className="mt-1 text-lg font-semibold text-slate-100">{value}</div>
    </div>
  );
}

function AircraftRow({ ac }: { ac: HangarAircraft }) {
  const model = ac.model ? cleanAtcType(ac.model) ?? ac.model : null;
  return (
    <div className="flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/60 p-3">
      <AirlineLogo icao={ac.airlineIcao} name={ac.airlineName} size={36} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-semibold text-slate-100">
            {model ?? t("hangar.unknown_model")}
          </span>
          {ac.registration && (
            <span className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[10px] text-slate-300">
              {ac.registration}
            </span>
          )}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-slate-500">
          {ac.airlineName && <span className="truncate">{ac.airlineName}</span>}
          <span>
            {ac.flights} {t("hangar.flights_short")}
          </span>
          <span>{Math.round(ac.totalNm).toLocaleString()} nm</span>
          <span>{fmtHours(ac.totalTimeS)}</span>
        </div>
      </div>
      <HealthBadge ac={ac} />
    </div>
  );
}

function HealthBadge({ ac }: { ac: HangarAircraft }) {
  const cfg = {
    good: {
      icon: <ShieldCheck className="h-3.5 w-3.5" />,
      cls: "bg-brand-500/15 text-brand-300 ring-brand-500/30",
      label: t("hangar.health.good"),
    },
    watch: {
      icon: <Wrench className="h-3.5 w-3.5" />,
      cls: "bg-amber-500/15 text-amber-300 ring-amber-500/30",
      label: t("hangar.health.watch"),
    },
    alert: {
      icon: <AlertTriangle className="h-3.5 w-3.5" />,
      cls: "bg-rose-500/15 text-rose-300 ring-rose-500/30",
      label: t("hangar.health.alert"),
    },
  }[ac.health] ?? {
    icon: <ShieldCheck className="h-3.5 w-3.5" />,
    cls: "bg-slate-800 text-slate-400 ring-slate-700",
    label: ac.health,
  };

  const fpm =
    ac.avgLandingFpm != null ? `${Math.round(ac.avgLandingFpm)} fpm` : "—";

  return (
    <div className="flex shrink-0 flex-col items-end gap-1">
      <span
        className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ${cfg.cls}`}
      >
        {cfg.icon}
        {cfg.label}
      </span>
      <span
        className="inline-flex items-center gap-1 text-[10px] text-slate-500"
        title={t("hangar.avg_touchdown")}
      >
        <Gauge className="h-3 w-3" />
        {fpm}
        {ac.hardLandings > 0 && (
          <span className="text-amber-400">· {ac.hardLandings} ⚠</span>
        )}
      </span>
    </div>
  );
}

function fmtHours(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m.toString().padStart(2, "0")}m`;
}
