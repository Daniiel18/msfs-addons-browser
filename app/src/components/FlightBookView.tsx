import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { t } from "../lib/i18n";
import {
  AlertCircle,
  ArrowLeft,
  Clock,
  Droplet,
  Globe,
  MapPin,
  Package,
  Plane,
  PlaneLanding,
  PlaneTakeoff,
  Ruler,
  Trash2,
  TrendingDown,
  Users,
} from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import type { FlightLogEntry } from "../lib/types";
import { RoutesMapView } from "./RoutesMapView";
import { EditFlightModal } from "./EditFlightModal";
import { Pencil } from "lucide-react";

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
  const lastError = useFlightLogStore((s) => s.lastError);
  const reload = useFlightLogStore((s) => s.reload);
  const remove = useFlightLogStore((s) => s.remove);

  // Selección de vuelo para mostrar su track real en el mapa.
  // null/undefined = modo globo (todos los vuelos a la vez).
  const [selectedFlightId, setSelectedFlightId] = useState<number | null>(null);
  // (v3.2.0) Modal de confirmación de borrado — reemplaza window.confirm
  // por una UI estética. `null` = oculto. Guardamos el entry completo
  // para mostrar contexto (orig→dest + fecha) y el id para llamar
  // remove() al confirmar.
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  const selectedFlight = useMemo(
    () =>
      selectedFlightId != null
        ? entries.find((e) => e.id === selectedFlightId) ?? null
        : null,
    [entries, selectedFlightId],
  );

  // Refresh al montar — el watcher emite eventos que actualizan
  // automáticamente, pero el primer mount necesita un pull.
  useEffect(() => {
    void reload();
  }, [reload]);

  // Si el vuelo seleccionado desaparece (lo borraron, recarga falló),
  // volvemos al modo globo en lugar de quedar con un id roto.
  useEffect(() => {
    if (selectedFlightId != null && !entries.find((e) => e.id === selectedFlightId)) {
      setSelectedFlightId(null);
    }
  }, [entries, selectedFlightId]);

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
    // (v2.2.0) FPM removido — la captura no era precisa.
    // v1.0.0: totales para los nuevos stat-cards de Pasajeros / Carga / Fuel.
    // Filtramos valores null antes de sumar — un vuelo viejo sin estos
    // campos no tira el total a NaN ni contamina la cuenta "X vuelos
    // con datos". Si NINGÚN vuelo reporta, mostramos "—" en el card.
    const passengersValid = completed
      .map((e) => e.passengers)
      .filter((v): v is number => v !== null);
    const cargoValid = completed
      .map((e) => e.cargoKg)
      .filter((v): v is number => v !== null);
    const fuelValid = completed
      .map((e) => e.fuelUsedKg)
      .filter((v): v is number => v !== null);
    const totalPassengers = passengersValid.reduce((a, b) => a + b, 0);
    const totalCargoKg = cargoValid.reduce((a, b) => a + b, 0);
    const totalFuelKg = fuelValid.reduce((a, b) => a + b, 0);
    return {
      count: completed.length,
      totalSec,
      totalDistance,
      totalPassengers,
      totalCargoKg,
      totalFuelKg,
      passengersFlightCount: passengersValid.length,
      cargoFlightCount: cargoValid.length,
      fuelFlightCount: fuelValid.length,
    };
  }, [completed]);

  return (
    <section className="space-y-5">
      <header className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <h1 className="flex items-center gap-2 text-lg font-semibold text-slate-100">
            <Plane className="h-5 w-5 text-emerald-300" />
            FlightBook
            {selectedFlight && (
              <span className="ml-1 inline-flex items-center gap-1 rounded-md bg-amber-500/15 px-2 py-0.5 text-xs font-medium text-amber-200 ring-1 ring-amber-500/30">
                <PlaneLanding className="h-3 w-3" />
                {selectedFlight.originIcao ?? "?"} →{" "}
                {selectedFlight.destinationIcao ?? "?"}
              </span>
            )}
          </h1>
          <p className="text-xs text-slate-500">
            {selectedFlight
              ? "Detalle del vuelo — ruta real grabada cada 10s."
              : "Bitácora de vuelos reales."}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {selectedFlight ? (
            <button
              onClick={() => setSelectedFlightId(null)}
              className="inline-flex items-center gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-xs font-medium text-amber-200 hover:border-amber-400 hover:bg-amber-500/20"
              title="Volver a la vista de globo con todos los vuelos"
            >
              <ArrowLeft className="h-3 w-3" />
              <Globe className="h-3 w-3" />
              Volver al globo
            </button>
          ) : null}
          {/* (v1.1.4) Demo + Recargar removidos. El watcher emite
              `flightlog://changed` y el store refresca solo. */}
        </div>
      </header>

      {lastError && (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{lastError}</span>
        </div>
      )}

      {/* Layout split: mapa a la izq + panel/lista a la der.
          (v1.1.4) Columna derecha 480px para que el detalle del vuelo
          se lea cómodamente.
          (v3.1.2) Reducimos la escala del mapa: la columna derecha
          sube a 540px (era 480) para que la distribución texto/mapa
          quede más equilibrada. Altura cae a `100vh - 13rem` (era
          11rem) — deja ~32px de margen inferior limpio, mejor estética
          UI/UX según pedido del usuario. */}
      <div
        className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_540px]"
        style={{ minHeight: "calc(100vh - 13rem)" }}
      >
        <div className="lg:sticky lg:top-4 lg:self-start">
          <RoutesMapView
            height="calc(100vh - 13rem)"
            selectedFlightId={selectedFlightId}
          />
        </div>

        <div
          className="flex min-h-0 flex-col gap-3"
          style={{ maxHeight: "calc(100vh - 13rem)" }}
        >
          {/* Panel superior — adapta su contenido al modo:
              · Detalle: ficha completa del vuelo seleccionado.
              · Globo: stats agregados del histórico.
              Si no hay completados, ocultamos para no dejar
              cards vacías. */}
          {selectedFlight ? (
            <SelectedFlightPanel entry={selectedFlight} />
          ) : (
            stats && (
              <div className="grid grid-cols-2 gap-2">
                <StatCard
                  icon={<Plane className="h-4 w-4 text-emerald-300" />}
                  label="Flights"
                  value={stats.count.toString()}
                  hint="Block-to-block recorded"
                />
                <StatCard
                  icon={<Clock className="h-4 w-4 text-sky-300" />}
                  label="Total time"
                  value={formatHM(stats.totalSec)}
                  hint="Block-to-block summed"
                />
                <StatCard
                  icon={<Ruler className="h-4 w-4 text-violet-300" />}
                  label="Distance"
                  value={`${Math.round(stats.totalDistance).toLocaleString("en-US")} nm`}
                  hint={`${Math.round(stats.totalDistance * 1.852).toLocaleString(
                    "en-US",
                  )} km`}
                />
                {/* (v2.2.0) FPM removed — capture imprecise.
                    (v1.0.0) Totales de pasajeros, carga, combustible.
                    Cada uno con un hint "X de Y vuelos reportan" para
                    que el usuario sepa si la suma cubre el histórico
                    completo o sólo los vuelos editados/con GSX. */}
                <StatCard
                  icon={<Users className="h-4 w-4 text-emerald-300" />}
                  label="Passengers"
                  value={
                    stats.passengersFlightCount > 0
                      ? stats.totalPassengers.toLocaleString("en-US")
                      : "—"
                  }
                  hint={
                    stats.passengersFlightCount > 0
                      ? `${stats.passengersFlightCount} of ${stats.count} flights`
                      : "no data yet"
                  }
                />
                <StatCard
                  icon={<Package className="h-4 w-4 text-violet-300" />}
                  label="Cargo"
                  value={
                    stats.cargoFlightCount > 0
                      ? `${stats.totalCargoKg.toLocaleString("es-ES")} kg`
                      : "—"
                  }
                  hint={
                    stats.cargoFlightCount > 0
                      ? `${(stats.totalCargoKg / 1000).toFixed(1)} t total`
                      : "no data yet"
                  }
                />
                <div className="col-span-2">
                  <StatCard
                    icon={<Droplet className="h-4 w-4 text-sky-300" />}
                    label="Total fuel"
                    value={
                      stats.fuelFlightCount > 0
                        ? `${stats.totalFuelKg.toLocaleString("en-US")} kg`
                        : "—"
                    }
                    hint={
                      stats.fuelFlightCount > 0
                        ? `${(stats.totalFuelKg / 1000).toFixed(1)} t · ${
                            stats.fuelFlightCount
                          } of ${stats.count} flights`
                        : "auto-captured during OUT/IN"
                    }
                  />
                </div>
              </div>
            )
          )}

          {/* Active flight — sólo cuando NO estamos en detail mode
              de un completed (el inFlight es siempre distinto del
              selectedFlight porque el filtro de completed excluye
              `endedAt === null`).
              (v3.1.2) "Destination" ahora muestra el destino real
              del OFP de SimBrief matching el origen actual — antes
              hardcodeaba "en ruta…" que era el bug que el usuario
              reportó con la screenshot SEGU. */}
          {inFlight && !selectedFlight && (
            <ActiveFlightCard entry={inFlight} />
          )}

          {/* Lista compacta de vuelos (cards verticales en lugar de
              tabla) — más legible cuando vive en una columna de 440px
              al lado del mapa. Scroll interno + max-height para que
              no salga del viewport. */}
          {completed.length === 0 ? (
            <div className="rounded-xl border border-dashed border-slate-700 bg-slate-900/40 p-6 text-center">
              <Plane className="mx-auto mb-2 h-7 w-7 text-slate-600" />
              <p className="text-sm font-medium text-slate-300">
                No flights logged yet
              </p>
              <p className="mt-1 text-[11px] text-slate-500">
                When you take off in MSFS with SimConnect active, the
                watcher will create an entry automatically.
              </p>
            </div>
          ) : (
            <ul className="min-h-0 flex-1 space-y-1.5 overflow-y-auto pr-1">
              {completed.map((e) => (
                <FlightCard
                  key={e.id}
                  entry={e}
                  selected={e.id === selectedFlightId}
                  onSelect={() =>
                    setSelectedFlightId((prev) => (prev === e.id ? null : e.id))
                  }
                  onDelete={() => {
                    // (v3.2.0) Abre modal estético en vez del feo
                    // window.confirm nativo. El modal lee el id del
                    // state y llama remove() al confirmar.
                    setConfirmDeleteId(e.id);
                  }}
                />
              ))}
            </ul>
          )}
        </div>
      </div>

      {/* (v3.2.0) Modal de confirmación de borrado. Estético en vez
          del feo `window.confirm`. */}
      {confirmDeleteId != null && (() => {
        const target = entries.find((e) => e.id === confirmDeleteId);
        if (!target) {
          // Edge case: entry desapareció (refresh paralelo).
          setConfirmDeleteId(null);
          return null;
        }
        return (
          <DeleteFlightModal
            entry={target}
            onConfirm={() => {
              void remove(target.id);
              setConfirmDeleteId(null);
            }}
            onCancel={() => setConfirmDeleteId(null)}
          />
        );
      })()}
    </section>
  );
}

/** (v3.2.0) Modal estético de confirmación al borrar un vuelo del
 *  FlightBook. Reemplaza el `window.confirm` nativo (UI del SO) por
 *  un overlay con AnimatePresence + Tailwind, manteniendo el theme
 *  de la app. Botones rojos (Eliminar) y neutro (Cancelar). */
function DeleteFlightModal({
  entry,
  onConfirm,
  onCancel,
}: {
  entry: FlightLogEntry;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const orig = entry.originIcao ?? "?";
  const dest = entry.destinationIcao ?? "?";
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/75 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm rounded-2xl border border-rose-500/30 bg-slate-900 shadow-2xl shadow-rose-500/10"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center gap-2 border-b border-slate-800 px-5 py-3">
          <div className="flex h-7 w-7 items-center justify-center rounded-full bg-rose-500/15 ring-1 ring-rose-500/30">
            <Trash2 className="h-3.5 w-3.5 text-rose-300" />
          </div>
          <h3 className="text-sm font-semibold text-slate-100">
            Delete flight?
          </h3>
        </header>
        <div className="space-y-3 px-5 py-4">
          <div className="rounded-md border border-slate-800 bg-slate-950/50 px-3 py-2">
            <div className="font-mono text-sm text-slate-100">
              {orig}
              <span className="mx-1.5 text-rose-300">→</span>
              {dest}
            </div>
            <div className="mt-0.5 text-[11px] text-slate-500">
              {formatDate(entry.startedAt)}
              {entry.aircraftAtcType && ` · ${entry.aircraftAtcType}`}
            </div>
          </div>
          <p className="text-xs text-slate-400">
            This action cannot be undone. The flight, its full track
            (live position points) and any associated edits will be
            permanently removed from the FlightBook.
          </p>
        </div>
        <footer className="flex justify-end gap-2 border-t border-slate-800 bg-slate-950/30 px-5 py-3">
          <button
            onClick={onCancel}
            className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-600 hover:text-slate-100"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="inline-flex items-center gap-1 rounded-md bg-rose-500/80 px-3 py-1.5 text-xs font-semibold text-rose-50 hover:bg-rose-500"
          >
            <Trash2 className="h-3 w-3" />
            Delete
          </button>
        </footer>
      </div>
    </div>
  );
}

/**
 * Panel de detalle del vuelo seleccionado. Reemplaza al StatGrid
 * agregado cuando el usuario clica un vuelo concreto — los stats
 * agregados (totales de todos los vuelos) son irrelevantes cuando
 * estás mirando UN vuelo específico, así que aquí ponemos sus
 * propias métricas: duración block-to-block, distancia, max alt,
 * max GS, landing FPM coloreado.
 */
/** (v3.3.0) Panel de detalle del vuelo seleccionado — rediseño
 *  completo con bloques organizados estilo "aviation log" moderno.
 *
 *  Estructura:
 *    · Header sticky: ruta XL + aerolínea/aeronave + acciones.
 *    · Bloque 1 — Route: orig, dest, alterno (si hay), distance, gates.
 *    · Bloque 2 — Times: OUT, IN, Block. (OFF/ON pendiente de captura
 *      en el watcher; mostramos lo que tenemos honestamente.)
 *    · Bloque 3 — Load (SimBrief): passengers, cargo, fuel.
 *    · Bloque 4 — Aircraft: type, registration (placeholder),
 *      airline (placeholder).
 *
 *  Cada bloque es una card independiente con borde redondeado, shadow
 *  sutil y padding generoso. AnimatePresence + motion.div para
 *  microanimaciones al cambiar de vuelo seleccionado. */
function SelectedFlightPanel({ entry }: { entry: FlightLogEntry }) {
  const [editing, setEditing] = useState(false);
  const reload = useFlightLogStore((s) => s.reload);

  // (v3.3.0) Re-renderiza con animación al cambiar de vuelo.
  return (
    <AnimatePresence mode="wait">
      <motion.div
        key={entry.id}
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -8 }}
        transition={{ duration: 0.2 }}
        className="space-y-3"
      >
        {/* HEADER — Card prominente con ruta y aerolínea. */}
        <div className="rounded-2xl border border-amber-500/40 bg-gradient-to-br from-amber-500/[0.12] to-amber-500/[0.04] p-4 shadow-md shadow-amber-500/10 ring-1 ring-amber-500/20">
          <div className="mb-2 flex items-center justify-between gap-2">
            <div className="inline-flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-amber-300/90">
              <PlaneLanding className="h-3 w-3" />
              {t("fb.selected_flight")}
            </div>
            <button
              onClick={() => setEditing(true)}
              title={t("fb.action.edit")}
              className="inline-flex items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/10 px-2.5 py-1 text-[10px] font-medium text-amber-200 transition-colors hover:bg-amber-500/20"
            >
              <Pencil className="h-2.5 w-2.5" />
              {t("fb.action.edit")}
            </button>
          </div>
          {editing && (
            <EditFlightModal
              entry={entry}
              onClose={() => setEditing(false)}
              onSaved={() => {
                setEditing(false);
                void reload();
              }}
            />
          )}

          {/* ROUTE BIG — bold extreme, font-mono, hierarchy. */}
          <div className="flex items-baseline gap-3 font-mono">
            <span className="text-2xl font-bold tracking-wide text-amber-50">
              {entry.originIcao ?? "?"}
            </span>
            <span className="text-amber-400">→</span>
            <span className="text-2xl font-bold tracking-wide text-amber-50">
              {entry.destinationIcao ?? "?"}
            </span>
          </div>
          {/* AIRCRAFT + DATE — medium peso, secundario. */}
          <div className="mt-1.5 flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
            {(entry.aircraftAtcType || entry.aircraftTitle) && (
              <span className="text-sm font-medium text-amber-100">
                {entry.aircraftAtcType ?? entry.aircraftTitle}
              </span>
            )}
            <span className="text-[11px] tracking-wide text-amber-300/70">
              · {formatDate(entry.startedAt)}
            </span>
          </div>
        </div>

        {/* BLOQUE 1 — Route */}
        <DetailBlock title={t("fb.block.route")} icon="route">
          <BlockRow label={t("fb.route.origin")} value={
            <span className="font-mono">
              <span className="font-bold text-slate-100">
                {entry.originIcao ?? "?"}
              </span>
              {entry.originName && (
                <span className="ml-1.5 font-sans text-[11px] font-normal text-slate-400">
                  {entry.originName}
                </span>
              )}
            </span>
          } />
          <BlockRow label={t("fb.route.destination")} value={
            <span className="font-mono">
              <span className="font-bold text-slate-100">
                {entry.destinationIcao ?? "?"}
              </span>
              {entry.destinationName && (
                <span className="ml-1.5 font-sans text-[11px] font-normal text-slate-400">
                  {entry.destinationName}
                </span>
              )}
            </span>
          } />
          <BlockRow label={t("fb.route.distance")} value={
            entry.distanceNm !== null
              ? `${Math.round(entry.distanceNm).toLocaleString("en-US")} nm`
              : "—"
          } />
          {entry.departureGate && (
            <BlockRow
              label={t("fb.route.dep_gate")}
              value={shortGate(entry.departureGate)}
            />
          )}
          {entry.arrivalGate && (
            <BlockRow
              label={t("fb.route.arr_gate")}
              value={shortGate(entry.arrivalGate)}
            />
          )}
        </DetailBlock>

        {/* BLOQUE 2 — Times (OUT/IN/Block; OFF/ON when available) */}
        <DetailBlock title={t("fb.block.times")} icon="times">
          <BlockRow
            label={t("fb.times.out")}
            value={
              <span className="font-mono">{formatTime(entry.startedAt)}</span>
            }
          />
          <BlockRow
            label={t("fb.times.in")}
            value={
              entry.endedAt ? (
                <span className="font-mono">{formatTime(entry.endedAt)}</span>
              ) : (
                "—"
              )
            }
          />
          <BlockRow
            label={t("fb.times.block")}
            value={
              <span className="font-mono font-semibold text-slate-100">
                {entry.flightTimeS !== null
                  ? formatHM(entry.flightTimeS)
                  : "—"}
              </span>
            }
          />
          {entry.pausedSeconds > 0 && (
            <BlockRow
              label={t("fb.times.paused")}
              value={
                <span className="font-mono text-[11px] text-slate-400">
                  {formatHM(entry.pausedSeconds)}
                </span>
              }
            />
          )}
        </DetailBlock>

        {/* BLOQUE 3 — Load (SimBrief) */}
        <DetailBlock title={t("fb.block.load")} icon="load">
          <BlockRow
            label={t("fb.load.passengers")}
            value={
              entry.passengers != null
                ? entry.passengers.toLocaleString("en-US")
                : "—"
            }
          />
          <BlockRow
            label={t("fb.load.cargo")}
            value={
              entry.cargoKg != null
                ? `${entry.cargoKg.toLocaleString("en-US")} kg`
                : "—"
            }
          />
          <BlockRow
            label={t("fb.load.fuel_used")}
            value={
              entry.fuelUsedKg != null
                ? `${entry.fuelUsedKg.toLocaleString("en-US")} kg`
                : "—"
            }
          />
        </DetailBlock>

        {/* BLOQUE 4 — Aircraft */}
        <DetailBlock title={t("fb.block.aircraft")} icon="aircraft">
          <BlockRow
            label={t("fb.aircraft.type")}
            value={entry.aircraftAtcType ?? "—"}
          />
          <BlockRow
            label={t("fb.aircraft.title")}
            value={entry.aircraftTitle ?? "—"}
          />
        </DetailBlock>

        {/* Métricas extra del vuelo (max alt / max speed) — compactas
            al final, ya no en grid prominente. */}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-2.5 text-[11px]">
          <span className="text-slate-500">Peak metrics:</span>
          {entry.maxAltitudeFt !== null && (
            <span className="font-mono text-slate-200">
              {entry.maxAltitudeFt.toLocaleString("en-US")} ft
            </span>
          )}
          {(entry.maxGroundSpeedKt !== null ||
            entry.maxTrueAirspeedKt !== null) && (
            <span className="font-mono text-slate-200">
              {entry.maxGroundSpeedKt ?? entry.maxTrueAirspeedKt} kt
            </span>
          )}
        </div>
      </motion.div>
    </AnimatePresence>
  );
}

/** (v3.3.0) Card de bloque para el detalle del vuelo. Borde
 *  redondeado, shadow sutil, padding generoso. Header con icono +
 *  título en uppercase tracking. Children son las rows. */
function DetailBlock({
  title,
  icon,
  children,
}: {
  title: string;
  icon: "route" | "times" | "load" | "aircraft";
  children: React.ReactNode;
}) {
  const Icon = ({}: Record<string, never>) => {
    switch (icon) {
      case "route":
        return <MapPin className="h-3.5 w-3.5 text-sky-300" />;
      case "times":
        return <Clock className="h-3.5 w-3.5 text-emerald-300" />;
      case "load":
        return <Package className="h-3.5 w-3.5 text-violet-300" />;
      case "aircraft":
        return <Plane className="h-3.5 w-3.5 text-amber-300" />;
    }
  };
  return (
    <div className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4 shadow-sm transition-shadow hover:shadow-md">
      <div className="mb-2.5 inline-flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-slate-400">
        <Icon />
        {title}
      </div>
      <div className="space-y-1.5">{children}</div>
    </div>
  );
}

/** (v3.3.0) Row de un bloque — label izquierda en slate-500, value
 *  derecha en slate-100. Padding vertical generoso para legibilidad. */
function BlockRow({
  label,
  value,
}: {
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[12px]">
      <span className="shrink-0 text-slate-500">{label}</span>
      <span className="text-right text-slate-100">{value}</span>
    </div>
  );
}

/** Formatea HH:MM UTC desde un ISO timestamp. */
function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const hh = d.getUTCHours().toString().padStart(2, "0");
    const mm = d.getUTCMinutes().toString().padStart(2, "0");
    return `${hh}:${mm}Z`;
  } catch {
    return "—";
  }
}

/** (v3.1.2) Card del vuelo activo. Cruza el `originIcao` del entry
 *  con el OFP más reciente de SimBrief para mostrar el destino real
 *  (antes hardcodeaba "en ruta…"). Si no hay match, mostramos "?"
 *  hasta que el sync ocurra. */
function ActiveFlightCard({ entry }: { entry: FlightLogEntry }) {
  const simbriefFlights = useSimBriefStore((s) => s.flights);
  const matchedOfp = (() => {
    if (!entry.originIcao || !simbriefFlights.length) return null;
    const sorted = [...simbriefFlights].sort((a, b) => {
      const aTs = a.generatedAt ? parseInt(a.generatedAt, 10) : 0;
      const bTs = b.generatedAt ? parseInt(b.generatedAt, 10) : 0;
      return bTs - aTs;
    });
    return sorted.find((f) => f.originIcao === entry.originIcao) ?? null;
  })();
  const destIcao =
    entry.destinationIcao ?? matchedOfp?.destinationIcao ?? null;
  return (
    <div className="rounded-xl border border-emerald-500/40 bg-emerald-500/10 px-3 py-2.5 ring-1 ring-emerald-500/20">
      <div className="mb-1 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-emerald-300">
        <span className="relative flex h-2 w-2">
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
          <span className="relative inline-flex h-2 w-2 rounded-full bg-emerald-400" />
        </span>
        Flying now
      </div>
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 font-mono text-sm text-emerald-100">
        <span>
          <PlaneTakeoff className="mr-1 inline h-3.5 w-3.5" />
          {entry.originIcao ?? "?"}
        </span>
        <span className="text-emerald-300">→</span>
        <span>
          <PlaneLanding className="mr-1 inline h-3.5 w-3.5" />
          {destIcao ?? "?"}
        </span>
      </div>
      <div className="mt-0.5 text-[10px] text-emerald-300/80">
        {entry.aircraftAtcType ?? entry.aircraftTitle ?? "?"}
        {!entry.destinationIcao && matchedOfp && (
          <span className="ml-1.5 italic text-emerald-300/60">
            · destination from SimBrief OFP
          </span>
        )}
      </div>
    </div>
  );
}

// (v3.3.0) `Metric` legacy component eliminado — DetailBlock +
// BlockRow lo reemplazan completamente.

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

/** (v3.3.0) Card de vuelo rediseñada — premium look.
 *  · Bordes redondeados generosos (xl), shadow sutil al hover.
 *  · Jerarquía tipográfica:
 *      · ICAOs origen→destino: bold, tamaño grande, mono.
 *      · Aeronave / aerolínea: medium, slate-200.
 *      · Fecha + métricas: regular, slate-500, tamaño pequeño.
 *  · Selected: ring amber prominent + bg amber tenue. */
function FlightCard({
  entry,
  selected,
  onSelect,
  onDelete,
}: {
  entry: FlightLogEntry;
  selected: boolean;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const dateLabel = formatDate(entry.startedAt);
  const duration =
    entry.flightTimeS !== null ? formatHM(entry.flightTimeS) : "—";
  const distance =
    entry.distanceNm !== null
      ? `${Math.round(entry.distanceNm).toLocaleString("en-US")} nm`
      : null;
  const aircraftLabel =
    entry.aircraftAtcType ?? entry.aircraftTitle ?? null;

  return (
    <motion.li
      onClick={onSelect}
      whileHover={{ y: -1 }}
      transition={{ duration: 0.12 }}
      className={`group relative cursor-pointer overflow-hidden rounded-xl border p-3.5 transition-all ${
        selected
          ? "border-amber-500/60 bg-amber-500/[0.08] shadow-md shadow-amber-500/10 ring-1 ring-amber-500/20"
          : "border-slate-800 bg-slate-900/40 shadow-sm hover:border-slate-700 hover:bg-slate-900/60 hover:shadow-md"
      }`}
      title={
        selected
          ? "Selected flight — click to deselect"
          : "Click to view this flight's actual track on the map"
      }
    >
      {/* Ribbon accent left side cuando seleccionado */}
      {selected && (
        <span className="absolute left-0 top-0 h-full w-0.5 bg-amber-400" />
      )}

      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          {/* ROUTE — bold, mono, prominent. */}
          <div className="flex items-baseline gap-2 font-mono">
            <span
              className={`text-base font-bold tracking-wide ${
                selected ? "text-amber-100" : "text-slate-50"
              }`}
            >
              {entry.originIcao ?? "?"}
            </span>
            <span
              className={`text-sm ${selected ? "text-amber-400" : "text-emerald-400"}`}
            >
              →
            </span>
            <span
              className={`text-base font-bold tracking-wide ${
                selected ? "text-amber-100" : "text-slate-50"
              }`}
            >
              {entry.destinationIcao ?? "?"}
            </span>
          </div>
          {/* AIRCRAFT — medium peso, slate-200. */}
          {aircraftLabel && (
            <div className="mt-1 truncate text-[11px] font-medium text-slate-300">
              {aircraftLabel}
            </div>
          )}
          {/* DATE — regular, secundario. */}
          <div className="mt-0.5 text-[10px] tracking-wide text-slate-500">
            {dateLabel}
          </div>
        </div>
        <button
          onClick={(ev) => {
            ev.stopPropagation();
            onDelete();
          }}
          title="Delete entry"
          className="rounded p-1 text-slate-600 opacity-0 transition-colors hover:bg-rose-500/15 hover:text-rose-300 group-hover:opacity-100"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>

      {/* MÉTRICAS COMPACTAS — fila horizontal en lugar de grid 2x2. */}
      <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px]">
        <span className="inline-flex items-center gap-1 text-slate-400">
          <Clock className="h-2.5 w-2.5" />
          <span className="font-medium tabular-nums text-slate-200">
            {duration}
          </span>
        </span>
        {distance && (
          <span className="inline-flex items-center gap-1 text-slate-400">
            <Ruler className="h-2.5 w-2.5" />
            <span className="font-medium tabular-nums text-slate-200">
              {distance}
            </span>
          </span>
        )}
        {entry.maxAltitudeFt !== null && (
          <span className="inline-flex items-center gap-1 text-slate-400">
            <TrendingDown className="h-2.5 w-2.5 rotate-180" />
            <span className="font-medium tabular-nums text-slate-200">
              {entry.maxAltitudeFt.toLocaleString("en-US")} ft
            </span>
          </span>
        )}
      </div>
    </motion.li>
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
