import { useEffect, useMemo } from "react";
import {
  AlertCircle,
  Clock,
  Gauge,
  Loader2,
  MapPin,
  Plane,
  PlaneLanding,
  PlaneTakeoff,
  Plus,
  Ruler,
  Trash2,
  TrendingDown,
} from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import type { FlightLogEntry } from "../lib/types";
import { api } from "../lib/tauri";
import { RoutesMapView } from "./RoutesMapView";

/**
 * Vista completa del FlightBook — bitácora de vuelos reales
 * capturados por SimConnect.
 *
 * Tres niveles:
 *   1. **Resumen** arriba — totales y agregados.
 *   2. **Vuelo activo** (si lo hay) destacado con badge verde.
 *   3. **Tabla de historial** — un row por vuelo con todos los
 *      campos: ICAOs, gates, FPM, max speed, distancia, duración.
 *
 * El watcher (`simconnect_watcher`) inserta una fila al detectar
 * despegue y la actualiza al detectar aterrizaje (con todas las
 * métricas calculadas en `handle_aircraft_data`).
 */
export function FlightBookView() {
  const entries = useFlightLogStore((s) => s.entries);
  const loading = useFlightLogStore((s) => s.loading);
  const lastError = useFlightLogStore((s) => s.lastError);
  const reload = useFlightLogStore((s) => s.reload);
  const remove = useFlightLogStore((s) => s.remove);

  // Refresh al montar — el watcher emite eventos que actualizan
  // automáticamente, pero el primer mount necesita un pull.
  useEffect(() => {
    void reload();
  }, [reload]);

  const inFlight = entries.find((e) => e.endedAt === null);
  const completed = entries.filter((e) => e.endedAt !== null);

  const stats = useMemo(() => {
    if (completed.length === 0) {
      return null;
    }
    const totalSec = completed.reduce(
      (acc, e) => acc + (e.flightTimeS ?? 0),
      0,
    );
    const totalDistance = completed.reduce(
      (acc, e) => acc + (e.distanceNm ?? 0),
      0,
    );
    const avgLandingFpm = (() => {
      const valid = completed
        .map((e) => e.landingFpm)
        .filter((v): v is number => v !== null);
      if (valid.length === 0) return null;
      return Math.round(valid.reduce((a, b) => a + b, 0) / valid.length);
    })();
    const greasers = completed.filter(
      (e) => e.landingFpm !== null && e.landingFpm > -300,
    ).length;
    return {
      count: completed.length,
      totalSec,
      totalDistance,
      avgLandingFpm,
      greasers,
    };
  }, [completed]);

  return (
    <section className="space-y-5">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="flex items-center gap-2 text-lg font-semibold text-slate-100">
            <Plane className="h-5 w-5 text-emerald-300" />
            FlightBook
          </h1>
          <p className="text-xs text-slate-500">
            Bitácora de vuelos reales — captura ICAOs, gates, duración, FPM al
            aterrizar y velocidad máxima vía SimConnect.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => api.debugSeedFlightLog().then(() => void reload())}
            className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-400 hover:border-slate-700 hover:text-slate-200"
            title="Inserta un vuelo demo EBBR → LEMD"
          >
            <Plus className="h-3 w-3" /> Demo
          </button>
          <button
            onClick={() => void reload()}
            disabled={loading}
            className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-slate-700 hover:text-slate-100 disabled:opacity-50"
          >
            {loading ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              "Recargar"
            )}
          </button>
        </div>
      </header>

      {lastError && (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{lastError}</span>
        </div>
      )}

      {/* Layout split: mapa a la izq, stats+lista a la der. En
          pantallas chicas (lg-) apila vertical. El mapa queda
          sticky para que se vea mientras scrolleas la tabla. */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-[1fr_440px]">
        <div className="lg:sticky lg:top-4 lg:self-start">
          <RoutesMapView height={520} />
        </div>

        <div className="space-y-4">
          {/* Summary cards en grid de 2 columnas en la columna derecha */}
          {stats && (
            <div className="grid grid-cols-2 gap-2.5">
              <StatCard
                icon={<Plane className="h-4 w-4 text-emerald-300" />}
                label="Vuelos"
                value={stats.count.toString()}
                hint={`${stats.greasers} aterrizajes finos`}
              />
              <StatCard
                icon={<Clock className="h-4 w-4 text-sky-300" />}
                label="Tiempo total"
                value={formatHM(stats.totalSec)}
                hint="Suma horas+minutos"
              />
              <StatCard
                icon={<Ruler className="h-4 w-4 text-violet-300" />}
                label="Distancia"
                value={`${Math.round(stats.totalDistance).toLocaleString("es-ES")} nm`}
                hint={`${Math.round(stats.totalDistance * 1.852).toLocaleString(
                  "es-ES",
                )} km`}
              />
              <StatCard
                icon={<TrendingDown className="h-4 w-4 text-amber-300" />}
                label="FPM medio"
                value={
                  stats.avgLandingFpm !== null
                    ? `${stats.avgLandingFpm} fpm`
                    : "—"
                }
                hint="al touchdown"
              />
            </div>
          )}

      {/* Active flight */}
          {inFlight && (
            <div className="rounded-xl border border-emerald-500/40 bg-emerald-500/10 px-3 py-2.5 ring-1 ring-emerald-500/20">
              <div className="mb-1 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-emerald-300">
                <span className="relative flex h-2 w-2">
                  <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
                  <span className="relative inline-flex h-2 w-2 rounded-full bg-emerald-400" />
                </span>
                En vuelo ahora
              </div>
              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 font-mono text-sm text-emerald-100">
                <span>
                  <PlaneTakeoff className="mr-1 inline h-3.5 w-3.5" />
                  {inFlight.originIcao ?? "?"}
                </span>
                <span className="text-emerald-300">→</span>
                <span>
                  <PlaneLanding className="mr-1 inline h-3.5 w-3.5" />
                  en ruta…
                </span>
              </div>
              <div className="mt-0.5 text-[10px] text-emerald-300/80">
                {inFlight.aircraftAtcType ?? inFlight.aircraftTitle ?? "?"}
              </div>
            </div>
          )}

          {/* Lista compacta de vuelos (cards verticales en lugar de
              tabla) — más legible cuando vive en una columna de 440px
              al lado del mapa. Scroll interno + max-height para que
              no salga del viewport. */}
          {completed.length === 0 ? (
            <div className="rounded-xl border border-dashed border-slate-700 bg-slate-900/40 p-6 text-center">
              <Plane className="mx-auto mb-2 h-7 w-7 text-slate-600" />
              <p className="text-sm font-medium text-slate-300">
                Aún no hay vuelos registrados
              </p>
              <p className="mt-1 text-[11px] text-slate-500">
                Cuando despegues en MSFS con SimConnect activo, el watcher
                creará una entrada automáticamente. Mientras tanto puedes
                poblar con el botón "Demo".
              </p>
            </div>
          ) : (
            <ul className="max-h-[calc(100vh-22rem)] space-y-1.5 overflow-y-auto pr-1">
              {completed.map((e) => (
                <FlightCard
                  key={e.id}
                  entry={e}
                  onDelete={() => void remove(e.id)}
                />
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  );
}

function StatCard({
  icon,
  label,
  value,
  hint,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wide text-slate-500">
        {icon}
        {label}
      </div>
      <div className="mt-1 text-lg font-semibold text-slate-100">{value}</div>
      {hint && <div className="text-[10px] text-slate-500">{hint}</div>}
    </div>
  );
}

function FlightCard({
  entry,
  onDelete,
}: {
  entry: FlightLogEntry;
  onDelete: () => void;
}) {
  const dateLabel = formatDate(entry.startedAt);
  const duration =
    entry.flightTimeS !== null ? formatHM(entry.flightTimeS) : "—";
  const distance =
    entry.distanceNm !== null
      ? `${Math.round(entry.distanceNm).toLocaleString("es-ES")} nm`
      : null;
  const maxAlt =
    entry.maxAltitudeFt !== null
      ? `${entry.maxAltitudeFt.toLocaleString("es-ES")} ft`
      : null;
  const maxSpeed =
    entry.maxGroundSpeedKt !== null
      ? `${entry.maxGroundSpeedKt} kt`
      : entry.maxTrueAirspeedKt !== null
        ? `${entry.maxTrueAirspeedKt} kt`
        : null;
  const fpm = entry.landingFpm;
  const fpmLabel = fpm !== null ? `${fpm} fpm` : null;
  const fpmTone =
    fpm === null
      ? "text-slate-500"
      : fpm > -300
        ? "text-emerald-300"
        : fpm > -600
          ? "text-sky-300"
          : fpm > -1000
            ? "text-amber-300"
            : "text-rose-300";

  return (
    <li className="group rounded-lg border border-slate-800 bg-slate-900/40 p-2.5 hover:border-slate-700">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 font-mono text-sm text-slate-100">
            <span>{entry.originIcao ?? "?"}</span>
            <span className="text-emerald-300">→</span>
            <span>{entry.destinationIcao ?? "?"}</span>
          </div>
          <div className="mt-0.5 text-[10px] text-slate-500">
            {dateLabel}
            {entry.aircraftAtcType && (
              <>
                {" · "}
                {entry.aircraftAtcType}
              </>
            )}
          </div>
        </div>
        <button
          onClick={onDelete}
          title="Eliminar entrada"
          className="rounded p-0.5 text-slate-600 opacity-0 hover:bg-rose-500/15 hover:text-rose-300 group-hover:opacity-100"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>
      <div className="mt-1.5 grid grid-cols-2 gap-x-3 gap-y-0.5 text-[10px] text-slate-400">
        <div className="flex items-center gap-1">
          <Clock className="h-2.5 w-2.5 text-slate-500" />
          <span className="tabular-nums text-slate-300">{duration}</span>
        </div>
        {distance && (
          <div className="flex items-center gap-1">
            <Ruler className="h-2.5 w-2.5 text-slate-500" />
            <span className="tabular-nums text-slate-300">{distance}</span>
          </div>
        )}
        {maxAlt && (
          <div className="flex items-center gap-1">
            <TrendingDown className="h-2.5 w-2.5 rotate-180 text-slate-500" />
            <span className="tabular-nums text-slate-300">{maxAlt}</span>
          </div>
        )}
        {maxSpeed && (
          <div className="flex items-center gap-1">
            <Gauge className="h-2.5 w-2.5 text-slate-500" />
            <span className="tabular-nums text-slate-300">{maxSpeed}</span>
          </div>
        )}
        {fpmLabel && (
          <div className={`flex items-center gap-1 ${fpmTone}`}>
            <TrendingDown className="h-2.5 w-2.5" />
            <span className="tabular-nums">{fpmLabel}</span>
          </div>
        )}
      </div>
      {(entry.departureGate || entry.arrivalGate) && (
        <div className="mt-1 flex items-center gap-1 text-[10px] text-slate-500">
          <MapPin className="h-2.5 w-2.5" />
          {entry.departureGate && (
            <span title="Gate salida">{shortGate(entry.departureGate)}</span>
          )}
          {entry.arrivalGate && (
            <>
              <span>·</span>
              <span title="Gate llegada">
                {shortGate(entry.arrivalGate)}
              </span>
            </>
          )}
        </div>
      )}
    </li>
  );
}

/** Acorta el formato del gate fallback. Devuelve el string original
 *  cuando ya viene en un formato corto ("Stand · ...") o cuando no
 *  matchea ningún patrón conocido. */
function shortGate(gate: string): string {
  // Nuevo formato: "Stand · 320° 280m de EBBR" — ya es corto.
  if (gate.startsWith("Stand")) return gate;
  // Formato legacy: "Position: 50.9012, 4.4844"
  const m = gate.match(/^Position:\s*(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?)/);
  if (!m) return gate;
  const lat = Number(m[1]).toFixed(2);
  const lon = Number(m[2]).toFixed(2);
  return `${lat},${lon}`;
}

function formatHM(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0m";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m.toString().padStart(2, "0")}m`;
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleString("es-ES", {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso.slice(0, 10);
  }
}
