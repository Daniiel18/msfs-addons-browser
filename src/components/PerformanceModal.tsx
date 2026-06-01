import { useEffect, useMemo, useRef, useState } from "react";
import { motion } from "framer-motion";
import {
  ChartLine,
  Flame,
  Gauge,
  Loader2,
  Plane,
  Thermometer,
  X,
} from "lucide-react";
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { FlightTrackFullPoint } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (v4.0.0 — P5) Performance modal del FlightBook.
 *
 * Modal floating con tabs verticales para revisar la telemetría del
 * vuelo seleccionado en formato gráfico (Recharts). Es **resizable**
 * arrastrando la esquina inferior derecha — el usuario decide cuánto
 * espacio dar a los gráficos según resolución de pantalla.
 *
 * ## Tabs (orden visual)
 *
 *   1. **Vertical Profile** — Altitude (MSL) over time. Imprescindible
 *      para revisar climb/cruise/descent.
 *   2. **Speed Profile** — GS + IAS over time. Detecta overspeed,
 *      decel anormales.
 *   3. **Engines** — N1/N2 por slot. Hasta 4 líneas por gráfico.
 *   4. **Temps** — EGT + Oil Temperature.
 *   5. **Fuel Flow** — pph por motor.
 *   6. **Oil Pressure** — psi por motor.
 *
 * Cada chart usa `<ResponsiveContainer>` para llenar el área del tab.
 * Tooltip de Recharts custom-themed dark.
 *
 * ## Datos faltantes
 *
 * Para imports VAS-ACARS solo `altFt` + `gsKt` están populados —
 * el modal detecta series con todos los samples NULL para un slot
 * y oculta esa curva. Si TODOS los slots de un tab son NULL, el
 * tab muestra "datos no disponibles".
 *
 * ## Resizable
 *
 * Implementado con un handle SE absolutamente posicionado en la
 * esquina + listener de pointermove en `document` (cubre el caso
 * donde el cursor sale del modal mientras arrastra). Constraints:
 * min 480×360, max 95% viewport.
 */
export function PerformanceModal({
  flightId,
  onClose,
}: {
  flightId: number;
  onClose: () => void;
}) {
  const [points, setPoints] = useState<FlightTrackFullPoint[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<TabKey>("alt");

  // Tamaño manejado por React → re-render al resize. Default: 760×520
  // (cómodo para revisar Vertical Profile en un monitor estándar).
  const [size, setSize] = useState({ width: 760, height: 520 });
  const resizingRef = useRef<{ startX: number; startY: number; w: number; h: number } | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    setPoints(null);
    setError(null);
    api
      .getFlightTrackFull(flightId)
      .then((data) => {
        if (!cancelled) setPoints(data);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [flightId]);

  // Resize handlers globales — capture en document para que el drag
  // siga funcionando si el cursor sale del modal.
  useEffect(() => {
    const onMove = (ev: PointerEvent) => {
      const r = resizingRef.current;
      if (!r) return;
      const dx = ev.clientX - r.startX;
      const dy = ev.clientY - r.startY;
      const minW = 480;
      const minH = 360;
      const maxW = Math.floor(window.innerWidth * 0.95);
      const maxH = Math.floor(window.innerHeight * 0.95);
      setSize({
        width: Math.min(maxW, Math.max(minW, r.w + dx)),
        height: Math.min(maxH, Math.max(minH, r.h + dy)),
      });
    };
    const onUp = () => {
      resizingRef.current = null;
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    return () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    };
  }, []);

  const onResizeStart = (ev: React.PointerEvent) => {
    resizingRef.current = {
      startX: ev.clientX,
      startY: ev.clientY,
      w: size.width,
      h: size.height,
    };
    ev.preventDefault();
  };

  // ESC cierra.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Pre-procesa los samples convirtiendo `ts` a un epoch numérico
  // (Recharts XAxis usa números mejor que strings ISO). Si el sample
  // no tiene `ts` parseable, lo dropeamos.
  const data = useMemo(() => {
    if (!points) return [] as ChartRow[];
    const out: ChartRow[] = [];
    for (const p of points) {
      const tms = new Date(p.ts).getTime();
      if (!Number.isFinite(tms)) continue;
      out.push({ tms, ...p });
    }
    return out;
  }, [points]);

  // ¿Existe data para cada tab? Si no, el tab se etiqueta como
  // "Sin datos" y el chart muestra empty state.
  const availability = useMemo(() => computeAvailability(data), [data]);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      onClick={onClose}
      className="fixed inset-0 z-40 flex items-center justify-center bg-slate-950/60 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 8 }}
        transition={{ duration: 0.16 }}
        onClick={(e) => e.stopPropagation()}
        style={{ width: size.width, height: size.height }}
        className="relative flex flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur-xl"
      >
        {/* Header */}
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-slate-800 px-4 py-3">
          <div className="flex items-center gap-2">
            <ChartLine className="h-4 w-4 text-sky-300" />
            <h2 className="text-sm font-semibold text-slate-100">
              {t("fb.performance.title")}
            </h2>
            {points && (
              <span className="ml-1 rounded bg-slate-800 px-1.5 py-0.5 text-[10px] text-slate-400">
                {points.length} {t("fb.performance.samples")}
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            title={t("common.close")}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {/* Body: tabs sidebar + chart area */}
        <div className="flex min-h-0 flex-1">
          {/* Tabs sidebar */}
          <nav className="flex w-32 shrink-0 flex-col gap-0.5 border-r border-slate-800 p-2">
            {TABS.map((t_) => (
              <button
                key={t_.key}
                onClick={() => setTab(t_.key)}
                disabled={!availability[t_.key]}
                title={
                  availability[t_.key]
                    ? undefined
                    : t("fb.performance.no_data_in_flight")
                }
                className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] transition-colors ${
                  tab === t_.key
                    ? "bg-sky-500/15 text-sky-100 ring-1 ring-sky-500/40"
                    : availability[t_.key]
                      ? "text-slate-300 hover:bg-slate-800/70 hover:text-slate-100"
                      : "text-slate-600 opacity-50"
                }`}
              >
                <t_.Icon className="h-3.5 w-3.5" />
                <span className="truncate">{t(t_.labelKey)}</span>
              </button>
            ))}
          </nav>

          {/* Chart area */}
          <div className="flex min-h-0 flex-1 flex-col p-3">
            {!points && !error && (
              <div className="flex flex-1 items-center justify-center text-xs text-slate-500">
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("fb.performance.loading")}
              </div>
            )}
            {error && (
              <div className="m-auto max-w-md rounded-lg bg-rose-500/10 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
                {error}
              </div>
            )}
            {points && (
              <ChartForTab tab={tab} data={data} availability={availability} />
            )}
          </div>
        </div>

        {/* Resize handle SE */}
        <div
          onPointerDown={onResizeStart}
          className="absolute bottom-0 right-0 h-4 w-4 cursor-se-resize"
          style={{
            background:
              "linear-gradient(135deg, transparent 50%, rgba(100,116,139,0.5) 50%)",
          }}
          title={t("fb.performance.resize")}
        />
      </motion.div>
    </motion.div>
  );
}

type TabKey = "alt" | "spd" | "eng" | "temp" | "ff" | "oil";

const TABS: { key: TabKey; labelKey: string; Icon: typeof ChartLine }[] = [
  { key: "alt", labelKey: "fb.performance.tab.alt", Icon: Plane },
  { key: "spd", labelKey: "fb.performance.tab.spd", Icon: Gauge },
  { key: "eng", labelKey: "fb.performance.tab.eng", Icon: ChartLine },
  { key: "temp", labelKey: "fb.performance.tab.temp", Icon: Thermometer },
  { key: "ff", labelKey: "fb.performance.tab.ff", Icon: Flame },
  { key: "oil", labelKey: "fb.performance.tab.oil", Icon: Thermometer },
];

type ChartRow = FlightTrackFullPoint & { tms: number };

type Availability = Record<TabKey, boolean>;

/**
 * Detecta para CADA tab si hay al menos un sample con datos
 * no-NULL. Para Engines/Temps/FF/Oil bastante con que un slot
 * tenga datos para que el tab sea relevante.
 */
function computeAvailability(data: ChartRow[]): Availability {
  const out: Availability = {
    alt: false,
    spd: false,
    eng: false,
    temp: false,
    ff: false,
    oil: false,
  };
  for (const r of data) {
    if (r.altFt != null) out.alt = true;
    if (r.gsKt != null || r.iasKt != null) out.spd = true;
    if (
      r.engN11 != null ||
      r.engN12 != null ||
      r.engN13 != null ||
      r.engN14 != null
    )
      out.eng = true;
    if (
      r.engEgt1 != null ||
      r.engEgt2 != null ||
      r.engEgt3 != null ||
      r.engEgt4 != null ||
      r.engOilTemp1 != null ||
      r.engOilTemp2 != null ||
      r.engOilTemp3 != null ||
      r.engOilTemp4 != null
    )
      out.temp = true;
    if (
      r.engFfPph1 != null ||
      r.engFfPph2 != null ||
      r.engFfPph3 != null ||
      r.engFfPph4 != null
    )
      out.ff = true;
    if (
      r.engOilPress1 != null ||
      r.engOilPress2 != null ||
      r.engOilPress3 != null ||
      r.engOilPress4 != null
    )
      out.oil = true;
    // Early exit cuando todo está descubierto.
    if (Object.values(out).every(Boolean)) break;
  }
  return out;
}

/** Determina qué slots de motor tienen datos válidos para una métrica
 *  específica. Permite no pintar líneas planas en 0 cuando el slot
 *  no aplica (avión bimotor → slots 3 y 4 vacíos). */
function activeEngineSlots(
  data: ChartRow[],
  picker: (r: ChartRow) => (number | null)[],
): boolean[] {
  const slots = [false, false, false, false];
  for (const r of data) {
    const vals = picker(r);
    for (let i = 0; i < 4; i++) {
      if (vals[i] != null) slots[i] = true;
    }
    if (slots.every(Boolean)) break;
  }
  return slots;
}

function ChartForTab({
  tab,
  data,
  availability,
}: {
  tab: TabKey;
  data: ChartRow[];
  availability: Availability;
}) {
  if (!availability[tab]) {
    return (
      <div className="m-auto text-xs text-slate-500">
        {t("fb.performance.no_data")}
      </div>
    );
  }

  switch (tab) {
    case "alt":
      return <AltChart data={data} />;
    case "spd":
      return <SpdChart data={data} />;
    case "eng":
      return (
        <DualEngineChart
          data={data}
          unit="%"
          domain={[0, 110]}
          stack1={(r) => [r.engN11, r.engN12, r.engN13, r.engN14]}
          stack2={(r) => [r.engN21, r.engN22, r.engN23, r.engN24]}
          label1={t("fb.performance.series.n1")}
          label2={t("fb.performance.series.n2")}
        />
      );
    case "temp":
      return (
        <DualEngineChart
          data={data}
          unit="°C"
          domain={["auto", "auto"]}
          stack1={(r) => [r.engEgt1, r.engEgt2, r.engEgt3, r.engEgt4]}
          stack2={(r) => [
            r.engOilTemp1,
            r.engOilTemp2,
            r.engOilTemp3,
            r.engOilTemp4,
          ]}
          label1={t("fb.performance.series.egt")}
          label2={t("fb.performance.series.oil_temp")}
        />
      );
    case "ff":
      return (
        <EngineMultilineChart
          data={data}
          unit="pph"
          domain={[0, "auto"]}
          picker={(r) => [r.engFfPph1, r.engFfPph2, r.engFfPph3, r.engFfPph4]}
          labelPrefix={t("fb.performance.series.ff")}
        />
      );
    case "oil":
      return (
        <EngineMultilineChart
          data={data}
          unit="psi"
          domain={["auto", "auto"]}
          picker={(r) => [
            r.engOilPress1,
            r.engOilPress2,
            r.engOilPress3,
            r.engOilPress4,
          ]}
          labelPrefix={t("fb.performance.series.oil_press")}
        />
      );
  }
}

// ============================================================================
// Charts individuales
// ============================================================================

const AXIS_TICK_PROPS = {
  fontSize: 10,
  fill: "#94a3b8",
} as const;

const GRID_STROKE = "#1e293b";
const TOOLTIP_STYLE = {
  background: "rgba(2, 6, 23, 0.95)",
  border: "1px solid #334155",
  borderRadius: 8,
  color: "#e2e8f0",
  fontSize: 11,
} as const;

const ENG_COLORS = ["#38bdf8", "#10b981", "#f59e0b", "#a78bfa"];

function formatTime(tms: number): string {
  const d = new Date(tms);
  return `${d.getUTCHours().toString().padStart(2, "0")}:${d
    .getUTCMinutes()
    .toString()
    .padStart(2, "0")}`;
}

function AltChart({ data }: { data: ChartRow[] }) {
  return (
    <ResponsiveContainer width="100%" height="100%">
      <LineChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 4 }}>
        <CartesianGrid stroke={GRID_STROKE} strokeDasharray="3 3" />
        <XAxis
          dataKey="tms"
          type="number"
          scale="time"
          domain={["dataMin", "dataMax"]}
          tickFormatter={formatTime}
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
        />
        <YAxis
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
          tickFormatter={(v) => `${v.toLocaleString()} ft`}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          labelFormatter={(l) => formatTime(l as number)}
          formatter={(v) => `${(v as number).toLocaleString()} ft`}
        />
        <Line
          type="monotone"
          dataKey="altFt"
          name="Altitude (MSL)"
          stroke="#38bdf8"
          strokeWidth={1.5}
          dot={false}
          isAnimationActive={false}
        />
      </LineChart>
    </ResponsiveContainer>
  );
}

function SpdChart({ data }: { data: ChartRow[] }) {
  return (
    <ResponsiveContainer width="100%" height="100%">
      <LineChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 4 }}>
        <CartesianGrid stroke={GRID_STROKE} strokeDasharray="3 3" />
        <XAxis
          dataKey="tms"
          type="number"
          scale="time"
          domain={["dataMin", "dataMax"]}
          tickFormatter={formatTime}
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
        />
        <YAxis
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
          tickFormatter={(v) => `${v} kt`}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          labelFormatter={(l) => formatTime(l as number)}
        />
        <Legend wrapperStyle={{ fontSize: 11, color: "#cbd5e1" }} />
        <Line
          type="monotone"
          dataKey="gsKt"
          name="Groundspeed"
          stroke="#38bdf8"
          strokeWidth={1.5}
          dot={false}
          isAnimationActive={false}
        />
        <Line
          type="monotone"
          dataKey="iasKt"
          name="Airspeed (IAS)"
          stroke="#1e293b"
          strokeWidth={1.5}
          dot={false}
          isAnimationActive={false}
        />
      </LineChart>
    </ResponsiveContainer>
  );
}

/**
 * Chart con DOS stacks de hasta 4 líneas cada uno (N1+N2, EGT+Oil Temp).
 * Cada stack pinta los slots activos en su propio color. Útil para
 * comparar dos métricas relacionadas con el mismo eje Y.
 */
function DualEngineChart({
  data,
  unit,
  domain,
  stack1,
  stack2,
  label1,
  label2,
}: {
  data: ChartRow[];
  unit: string;
  domain: [number | string, number | string];
  stack1: (r: ChartRow) => (number | null)[];
  stack2: (r: ChartRow) => (number | null)[];
  label1: string;
  label2: string;
}) {
  const slots1 = useMemo(() => activeEngineSlots(data, stack1), [data, stack1]);
  const slots2 = useMemo(() => activeEngineSlots(data, stack2), [data, stack2]);
  // Transformamos los datos para que cada slot sea una key plana en
  // el objeto (Recharts no soporta nested accessors).
  const flat = useMemo(() => {
    return data.map((r) => {
      const v1 = stack1(r);
      const v2 = stack2(r);
      return {
        tms: r.tms,
        s1_1: v1[0],
        s1_2: v1[1],
        s1_3: v1[2],
        s1_4: v1[3],
        s2_1: v2[0],
        s2_2: v2[1],
        s2_3: v2[2],
        s2_4: v2[3],
      };
    });
  }, [data, stack1, stack2]);

  return (
    <ResponsiveContainer width="100%" height="100%">
      <LineChart data={flat} margin={{ top: 8, right: 12, left: 0, bottom: 4 }}>
        <CartesianGrid stroke={GRID_STROKE} strokeDasharray="3 3" />
        <XAxis
          dataKey="tms"
          type="number"
          scale="time"
          domain={["dataMin", "dataMax"]}
          tickFormatter={formatTime}
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
        />
        <YAxis
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
          domain={domain}
          tickFormatter={(v) => `${v}${unit}`}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          labelFormatter={(l) => formatTime(l as number)}
        />
        <Legend wrapperStyle={{ fontSize: 11, color: "#cbd5e1" }} />
        {slots1.map((on, i) =>
          on ? (
            <Line
              key={`s1_${i}`}
              type="monotone"
              dataKey={`s1_${i + 1}`}
              name={`${label1} #${i + 1}`}
              stroke={ENG_COLORS[i]}
              strokeWidth={1.2}
              dot={false}
              isAnimationActive={false}
            />
          ) : null,
        )}
        {slots2.map((on, i) =>
          on ? (
            <Line
              key={`s2_${i}`}
              type="monotone"
              dataKey={`s2_${i + 1}`}
              name={`${label2} #${i + 1}`}
              stroke={ENG_COLORS[i]}
              strokeDasharray="4 3"
              strokeWidth={1.2}
              dot={false}
              isAnimationActive={false}
            />
          ) : null,
        )}
      </LineChart>
    </ResponsiveContainer>
  );
}

/** Chart con una sola métrica × 4 slots (Fuel Flow, Oil Pressure). */
function EngineMultilineChart({
  data,
  unit,
  domain,
  picker,
  labelPrefix,
}: {
  data: ChartRow[];
  unit: string;
  domain: [number | string, number | string];
  picker: (r: ChartRow) => (number | null)[];
  labelPrefix: string;
}) {
  const slots = useMemo(() => activeEngineSlots(data, picker), [data, picker]);
  const flat = useMemo(() => {
    return data.map((r) => {
      const v = picker(r);
      return {
        tms: r.tms,
        s_1: v[0],
        s_2: v[1],
        s_3: v[2],
        s_4: v[3],
      };
    });
  }, [data, picker]);

  return (
    <ResponsiveContainer width="100%" height="100%">
      <LineChart data={flat} margin={{ top: 8, right: 12, left: 0, bottom: 4 }}>
        <CartesianGrid stroke={GRID_STROKE} strokeDasharray="3 3" />
        <XAxis
          dataKey="tms"
          type="number"
          scale="time"
          domain={["dataMin", "dataMax"]}
          tickFormatter={formatTime}
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
        />
        <YAxis
          tick={AXIS_TICK_PROPS}
          stroke={GRID_STROKE}
          domain={domain}
          tickFormatter={(v) => `${v} ${unit}`}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          labelFormatter={(l) => formatTime(l as number)}
        />
        <Legend wrapperStyle={{ fontSize: 11, color: "#cbd5e1" }} />
        {slots.map((on, i) =>
          on ? (
            <Line
              key={`s_${i}`}
              type="monotone"
              dataKey={`s_${i + 1}`}
              name={`${labelPrefix} #${i + 1}`}
              stroke={ENG_COLORS[i]}
              strokeWidth={1.2}
              dot={false}
              isAnimationActive={false}
            />
          ) : null,
        )}
      </LineChart>
    </ResponsiveContainer>
  );
}
