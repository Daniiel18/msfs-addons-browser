import { useCallback, useEffect, useMemo, useState } from "react";
import { X, Wrench, Plane, CheckCircle2, Activity } from "lucide-react";
import type {
  AircraftMaint,
  AirlineLedger,
  AirlinePolicy,
  RealTelemetry,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { AirlineLogo } from "./AirlineLogo";

const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumFractionDigits: 0,
});

/** Color por desgaste (0-100): verde → amarillo → ámbar → rojo. */
function wearColor(w: number): string {
  if (w >= 80) return "#ef4444";
  if (w >= 50) return "#f59e0b";
  if (w >= 25) return "#eab308";
  return "#22c55e";
}

const MAINT_LEVELS = [0, 1, 2] as const;

/** Normaliza una matrícula para comparar ("SE-RSC" → "SERSC"). */
const normReg = (s: string) => s.replace(/[^a-z0-9]/gi, "").toUpperCase();

/** Desgaste por ZONA de la silueta (máximo de los componentes de esa zona). */
export function zoneWearOf(components: { zone: string; wearPct: number }[]): Record<string, number> {
  const z: Record<string, number> = { engines: 0, gears: 0, fuselage: 0 };
  for (const c of components) z[c.zone] = Math.max(z[c.zone] ?? 0, c.wearPct);
  return z;
}

/** Categoría de fuselaje para dibujar la silueta correcta. */
function categoryOf(model?: string | null): "wide" | "narrow" | "regional" {
  const m = (model ?? "").toUpperCase();
  if (/(747|777|787|767|A30|A310|A330|A340|A350|A380|MD-?11|DC-?10|IL96)/.test(m)) return "wide";
  if (/(CRJ|ERJ|EMB|E1[0-9]5|E[12]9[05]|E-?JET|ATR|DASH|Q400|DHC|SAAB)/.test(m)) return "regional";
  return "narrow";
}

/* Siluetas top-view REALES (Peter Lowden, CC-BY, Wikimedia Commons). Se
 * renderizan SIN el flip vertical original → morro arriba. */
const PATH_737 =
  "m 32,1.9 0.3,0.1 0.8,1.2 0.9,3.1 0.6,3.1 0.5,5.2 0,9.1 2.9,2.4 -0.1,-1 0,-1.5 0.1,-1.2 0.2,-0.4 2.7,0 0.2,0.4 0.1,1.2 0,1.5 -0.3,2.3 -0.4,0 -0.1,0.3 1.8,1 15.9,8.3 0.9,0.8 0.5,1.2 0,1.9 -0.4,-0.9 -0.7,-0.9 -11.4,-3.6 -0.1,0.8 -0.3,0.7 -0.3,-0.7 -0.1,-1 -4.2,-1 -0.1,0.8 -0.3,0.7 -0.3,-0.7 -0.1,-1 -1.2,-0.2 -0.6,0 -0.1,0.9 -0.3,0.7 -0.3,-0.7 -0.1,-0.9 -3.5,0 0,11.6 -0.1,2.7 -0.3,2.5 -0.7,2.4 9.1,7 0,2 -10.7,-3.2 -0.3,1 -0.2,0 -0.3,-1 -10.7,3.2 0,-2 9.2,-7 -0.7,-2.4 -0.4,-2.5 -0.1,-2.7 0,-11.6 -3.5,0 -0.1,0.9 -0.3,0.7 -0.3,-0.7 -0.1,-0.9 -0.6,0 -1.2,0.2 -0.1,1 -0.3,0.7 -0.3,-0.7 -0.1,-0.8 -4.2,1 -0.1,1 -0.3,0.7 -0.3,-0.7 -0.1,-0.8 -11.4,3.6 -0.7,0.9 -0.4,0.9 0,-1.9 0.5,-1.2 0.9,-0.8 15.9,-8.3 1.8,-1 -0.1,-0.3 -0.5,0 -0.2,-2.3 0,-1.5 0.1,-1.2 0.2,-0.4 2.7,0 0.2,0.4 0.1,1.2 0,1.5 -0.1,1 2.9,-2.4 0,-9.1 0.5,-5.2 0.6,-3.1 1,-3.1 0.7,-1.2 z";
const PATH_777 =
  "m 32,0.2 0.1,0 0.2,0.1 0.3,0.3 0.5,0.8 0.7,1.7 0.5,2 0.3,2.5 0,15 0.3,0.6 0.5,0.6 3.3,2.2 -0.2,-1.1 0,-1.6 0.2,-1.4 0.1,-0.2 3,0 0.1,0.2 0.2,1.4 0,1.6 -0.2,1.4 -0.4,0 -0.4,1.3 16.4,10.8 0.2,0.5 0.1,1.5 -9,-3.1 -0.1,0.6 -0.1,0.2 -0.1,-0.2 -0.1,-0.7 -4.2,-1.4 -0.1,0.6 -0.1,0.2 -0.1,-0.2 -0.1,-0.7 -2.5,-0.8 -1.8,0 -0.1,0.7 -0.1,0.2 -0.1,-0.2 -0.1,-0.7 -4.5,0 0,14.9 -0.2,1.9 -0.7,3.8 7.6,6.2 0.1,0.7 0,0.7 -0.1,0.7 -8.5,-3.1 -0.2,0.4 -0.5,1.8 -0.2,0 -0.5,-1.8 -0.2,-0.4 -8.5,3.1 -0.1,-0.7 0,-0.7 0.1,-0.7 7.6,-6.2 -0.7,-3.8 -0.2,-1.9 0,-14.9 -4.5,0 -0.1,0.7 -0.1,0.2 -0.1,-0.2 -0.1,-0.7 -1.8,0 -2.5,0.8 -0.1,0.7 -0.1,0.2 -0.1,-0.2 -0.1,-0.6 -4.2,1.4 -0.1,0.7 -0.1,0.2 -0.1,-0.2 -0.1,-0.6 -9,3.1 0.1,-1.5 0.2,-0.5 16.4,-10.8 -0.4,-1.3 -0.4,0 -0.2,-1.4 0,-1.6 0.2,-1.4 0.1,-0.2 3,0 0.1,0.2 0.2,1.4 0,1.6 -0.2,1.1 3.3,-2.2 0.5,-0.6 0.3,-0.6 0,-15 0.3,-2.5 0.5,-2 0.7,-1.7 0.5,-0.8 0.3,-0.3 0.2,-0.1 z";

type Zone = [number, number, number]; // x, y, radio (coords 0-64)
const SILHOUETTES: Record<
  "narrow" | "wide" | "regional",
  { path: string; fuselage: { cx: number; cy: number; rx: number; ry: number }; engines: Zone[]; gears: Zone[] }
> = {
  narrow: {
    path: PATH_737,
    fuselage: { cx: 32, cy: 20, rx: 3, ry: 12 },
    engines: [[41, 30, 2.6], [23, 30, 2.6]],
    gears: [[35.5, 31.5, 1.6], [28.5, 31.5, 1.6], [32, 13, 1.5]],
  },
  wide: {
    path: PATH_777,
    fuselage: { cx: 32, cy: 22, rx: 3.5, ry: 13 },
    engines: [[43, 31, 3.2], [21, 31, 3.2]],
    gears: [[36, 33, 1.8], [28, 33, 1.8], [32, 14, 1.6]],
  },
  regional: {
    path: PATH_737,
    fuselage: { cx: 32, cy: 20, rx: 3, ry: 12 },
    engines: [[41, 30, 2.4], [23, 30, 2.4]],
    gears: [[35.5, 31.5, 1.5], [28.5, 31.5, 1.5], [32, 13, 1.4]],
  },
};

/**
 * (v6.1) Silueta TOP-VIEW con la imagen REAL del avión (737/777, Wikimedia) y
 * las zonas de mantenimiento (motores/trenes/fuselaje) encima: GRIS cuando el
 * componente está sano, ROJO según el desgaste (parpadea si toca servicio).
 */
export function Silhouette({
  zoneWear,
  model,
}: {
  zoneWear: Record<string, number>;
  model?: string | null;
}) {
  const z = (k: string) => zoneWear[k] ?? 0;
  const zf = (k: string) => {
    const w = z(k);
    const fill = w < 30 ? "#94a3b8" : wearColor(w);
    return { fill, fillOpacity: 0.25 + Math.min(0.45, (w / 100) * 0.5) };
  };
  const due = (k: string) => z(k) >= 80;
  const s = SILHOUETTES[categoryOf(model)];

  return (
    <svg viewBox="0 0 64 64" className="h-full w-full" xmlns="http://www.w3.org/2000/svg">
      <path d={s.path} fill="#334155" fillOpacity={0.28} stroke="#3b9eff" strokeWidth={0.45} strokeLinejoin="round" />
      {/* fuselaje */}
      <ellipse cx={s.fuselage.cx} cy={s.fuselage.cy} rx={s.fuselage.rx} ry={s.fuselage.ry} {...zf("fuselage")}>
        {due("fuselage") && <animate attributeName="fill-opacity" values="0.25;0.6;0.25" dur="1.1s" repeatCount="indefinite" />}
      </ellipse>
      {/* motores */}
      {s.engines.map(([x, y, r], i) => (
        <circle key={`e${i}`} cx={x} cy={y} r={r} {...zf("engines")}>
          {due("engines") && <animate attributeName="fill-opacity" values="0.3;0.75;0.3" dur="1.1s" repeatCount="indefinite" />}
        </circle>
      ))}
      {/* trenes (principales + morro) */}
      {s.gears.map(([x, y, r], i) => (
        <circle key={`g${i}`} cx={x} cy={y} r={r} {...zf("gears")}>
          {due("gears") && <animate attributeName="fill-opacity" values="0.3;0.8;0.3" dur="1s" repeatCount="indefinite" />}
        </circle>
      ))}
    </svg>
  );
}

/**
 * (v6.1) Vista de la silueta. USA LA IMAGEN del usuario si existe en
 * `public/aircraft/{narrow|wide|regional}.png` (basta soltar ahí sus PNG); si
 * no, cae al dibujo SVG. Sobre la imagen pinta indicadores de desgaste por
 * zona (motores/tren/fuselaje) para no perder la info de mantenimiento.
 */
export function SilhouetteView({
  zoneWear,
  model,
}: {
  zoneWear: Record<string, number>;
  model?: string | null;
}) {
  const cat = categoryOf(model);
  // Override opcional: si el usuario deja `public/aircraft/<cat>.png`, se usa
  // esa (precargada para no parpadear). Si no, la silueta real embebida.
  const [imgSrc, setImgSrc] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setImgSrc(null);
    const src = `/aircraft/${cat}.png`;
    const im = new Image();
    im.onload = () => alive && setImgSrc(src);
    im.onerror = () => alive && setImgSrc(null);
    im.src = src;
    return () => {
      alive = false;
    };
  }, [cat]);

  if (!imgSrc) return <Silhouette zoneWear={zoneWear} model={model} />;

  const dot = (k: string, label: string) => {
    const w = zoneWear[k] ?? 0;
    const col = w < 30 ? "#94a3b8" : wearColor(w);
    return (
      <div className="flex items-center gap-1.5">
        <span className="h-2.5 w-2.5 rounded-full" style={{ background: col }} />
        <span className="text-[10px] text-slate-400">{label} {Math.round(w)}%</span>
      </div>
    );
  };

  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-2">
      <img src={imgSrc} alt="" className="min-h-0 flex-1 object-contain" />
      <div className="flex flex-wrap items-center justify-center gap-x-3 gap-y-1">
        {dot("engines", t("economy.comp.engine_oil"))}
        {dot("gears", t("economy.comp.tires"))}
        {dot("fuselage", t("economy.comp.hydraulics"))}
      </div>
    </div>
  );
}

/** Componentes en orden EFB + su zona (sólo para fallback de zona). */
const COMP_ORDER = [
  "tires", "brakes", "engine_oil", "hydraulics",
  "fire_bottles", "oxygen", "egt", "idg",
];

/** (v6.1 datos reales) Tira con la telemetría REAL del sim que respalda el
 *  desgaste: ciclos, horas y picos capturados por SimConnect. El badge "live"
 *  confirma cuántos vuelos aportaron datos reales de motor. */
function TelemetryStrip({ tel }: { tel: RealTelemetry }) {
  const chips: { label: string; value: string; warn?: boolean }[] = [
    { label: t("economy.tel.cycles"), value: String(tel.cycles) },
    { label: t("economy.tel.hours"), value: `${tel.hours.toFixed(1)} h` },
  ];
  if (tel.peakEgtC != null)
    chips.push({ label: t("economy.tel.egt"), value: `${Math.round(tel.peakEgtC)}°C`, warn: tel.peakEgtC > 850 });
  if (tel.peakOilTempC != null)
    chips.push({ label: t("economy.tel.oil"), value: `${Math.round(tel.peakOilTempC)}°C`, warn: tel.peakOilTempC > 140 });
  if (tel.peakBrakeTempC != null)
    chips.push({ label: t("economy.tel.brake"), value: `${Math.round(tel.peakBrakeTempC)}°C`, warn: tel.peakBrakeTempC > 400 });
  if (tel.peakTireWearPct != null)
    chips.push({ label: t("economy.tel.tire"), value: `${Math.round(tel.peakTireWearPct)}%`, warn: tel.peakTireWearPct > 70 });
  if (tel.worstLandingFpm != null)
    chips.push({ label: t("economy.tel.land"), value: `${tel.worstLandingFpm} fpm`, warn: tel.worstLandingFpm < -600 });
  const hasReal = tel.flightsWithData > 0;

  return (
    <div className="mb-2 rounded-lg border border-slate-800 bg-slate-950/40 p-2">
      <div className="mb-1.5 flex items-center gap-1.5">
        <Activity className="h-3.5 w-3.5 text-emerald-400" />
        <span className="text-[11px] font-semibold text-slate-300">
          {t("economy.tel.heading")}
        </span>
        {hasReal ? (
          <span className="rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-emerald-300">
            {t("economy.tel.live", { n: tel.flightsWithData })}
          </span>
        ) : (
          <span className="rounded-full bg-slate-700/40 px-1.5 py-0.5 text-[9px] font-medium text-slate-400">
            {t("economy.tel.nodata")}
          </span>
        )}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {chips.map((c) => (
          <span
            key={c.label}
            className="flex items-center gap-1 rounded-md bg-slate-900/70 px-2 py-1 text-[10px]"
          >
            <span className="text-slate-500">{c.label}</span>
            <span className={`font-semibold ${c.warn ? "text-amber-300" : "text-slate-200"}`}>
              {c.value}
            </span>
          </span>
        ))}
      </div>
    </div>
  );
}

export function MaintenanceModal({
  led,
  initialReg,
  onClose,
  onChanged,
}: {
  led: AirlineLedger;
  initialReg?: string | null;
  onClose: () => void;
  onChanged: () => void | Promise<void>;
}) {
  const [fleet, setFleet] = useState<AircraftMaint[] | null>(null);
  const [policy, setPolicy] = useState<AirlinePolicy | null>(null);
  const [selected, setSelected] = useState<string | null>(initialReg ?? null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const [f, p] = await Promise.all([
      api.airlineFleetMaintenance(led.key),
      api.airlinePolicy(led.key),
    ]);
    setFleet(f);
    setPolicy(p);
    // Preselecciona la matrícula del Hangar (compara normalizada: "SE-RSC"
    // del Hangar vs la que devuelve la flota, con o sin guion).
    setSelected((cur) => {
      if (initialReg) {
        const m = f.find((a) => normReg(a.registration) === normReg(initialReg));
        if (m) return m.registration;
      }
      return cur ?? f[0]?.registration ?? null;
    });
  }, [led.key, initialReg]);

  useEffect(() => {
    void load();
  }, [load]);

  // Siempre hay un avión seleccionado si la flota no está vacía (evita el
  // "elige un avión" cuando se abre desde el Hangar con matrícula).
  const current = useMemo(
    () => fleet?.find((a) => a.registration === selected) ?? fleet?.[0] ?? null,
    [fleet, selected],
  );

  const zoneWear = useMemo(() => {
    const z: Record<string, number> = { engines: 0, gears: 0, fuselage: 0 };
    for (const c of current?.components ?? []) {
      z[c.zone] = Math.max(z[c.zone] ?? 0, c.wearPct);
    }
    return z;
  }, [current]);

  const setLevel = async (level: number) => {
    if (!policy) return;
    setBusy(true);
    try {
      const next = { ...policy, maintenanceLevel: level };
      await api.setAirlinePolicy(led.key, next);
      setPolicy(next);
      await load(); // recarga la flota → los precios reflejan el nuevo nivel
      await onChanged();
    } finally {
      setBusy(false);
    }
  };

  const doService = async (component: string) => {
    if (!current) return;
    setBusy(true);
    try {
      await api.serviceAircraft(current.registration, component, led.key);
      await load();
      await onChanged();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <div
        className="flex max-h-[88vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-slate-700 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* header */}
        <div className="flex items-center gap-3 border-b border-slate-800 p-4">
          <AirlineLogo icao={led.icao} name={led.name} size={32} />
          <div className="min-w-0 flex-1">
            <h3 className="flex items-center gap-1.5 truncate text-sm font-semibold text-slate-100">
              <Wrench className="h-4 w-4 text-amber-400" />
              {t("economy.maint.title")}
            </h3>
            <p className="truncate text-xs text-slate-500">{led.name}</p>
          </div>
          <button onClick={onClose} className="rounded-lg p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-300">
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* maintenance level */}
        <div className="flex flex-wrap items-center gap-2 border-b border-slate-800 px-4 py-2.5">
          <span className="text-xs text-slate-500">{t("economy.maint.level")}:</span>
          <div className="flex items-center gap-1 rounded-lg bg-slate-950/70 p-0.5 ring-1 ring-slate-800">
            {MAINT_LEVELS.map((lvl) => {
              const name = ["basic", "standard", "premium"][lvl];
              const on = policy?.maintenanceLevel === lvl;
              return (
                <button
                  key={lvl}
                  disabled={busy}
                  onClick={() => setLevel(lvl)}
                  className={`rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors disabled:opacity-50 ${
                    on ? "bg-brand-500/20 text-brand-200" : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {t(`economy.maint.${name}`)}
                </button>
              );
            })}
          </div>
        </div>

        {!fleet ? (
          <div className="py-20 text-center text-sm text-slate-500">{t("economy.maint.loading")}</div>
        ) : fleet.length === 0 ? (
          <div className="py-20 text-center text-sm text-slate-500">{t("economy.maint.empty")}</div>
        ) : (
          <div className="flex min-h-0 flex-1 overflow-hidden">
            {/* fleet list */}
            <div className="w-44 shrink-0 overflow-y-auto border-r border-slate-800 p-2">
              {fleet.map((a) => {
                const on = a.registration === selected;
                return (
                  <button
                    key={a.registration}
                    onClick={() => setSelected(a.registration)}
                    className={`mb-1.5 w-full rounded-xl border p-2 text-left transition-colors ${
                      on ? "border-brand-500/60 bg-brand-500/10" : "border-transparent bg-slate-950/40 hover:bg-slate-800/40"
                    }`}
                  >
                    <div className="flex items-center gap-1.5">
                      <Plane className="h-3 w-3 text-slate-500" />
                      <span className="truncate text-xs font-semibold text-slate-100">{a.registration}</span>
                    </div>
                    <div className="truncate text-[10px] text-slate-500">{a.model ?? "—"}</div>
                    <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                      <div className="h-full rounded-full" style={{ width: `${a.overallWear}%`, background: wearColor(a.overallWear) }} />
                    </div>
                  </button>
                );
              })}
            </div>

            {/* detail */}
            <div className="min-w-0 flex-1 overflow-y-auto p-4">
              {!current ? (
                <p className="py-10 text-center text-sm text-slate-500">{t("economy.maint.select")}</p>
              ) : (
                <div className="flex flex-col gap-4 md:flex-row">
                  {/* silhouette */}
                  <div className="mx-auto h-64 w-56 shrink-0 md:mx-0">
                    <SilhouetteView zoneWear={zoneWear} model={current.model} />
                  </div>
                  {/* components */}
                  <div className="min-w-0 flex-1">
                    <div className="mb-2 flex items-baseline justify-between">
                      <h4 className="text-sm font-semibold text-slate-100">{current.registration}</h4>
                      <span className="text-[11px] text-slate-500">
                        {t("economy.maint.flights", { n: current.flights })}
                      </span>
                    </div>
                    {current.telemetry && <TelemetryStrip tel={current.telemetry} />}
                    <div className="space-y-1.5">
                      {[...current.components]
                        .sort((a, b) => COMP_ORDER.indexOf(a.id) - COMP_ORDER.indexOf(b.id))
                        .map((c) => (
                          <div
                            key={c.id}
                            className="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-950/40 p-2"
                          >
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center justify-between">
                                <span className="truncate text-xs font-medium text-slate-200">
                                  {t(`economy.comp.${c.id}`)}
                                </span>
                                <span className="text-[11px] font-semibold" style={{ color: wearColor(c.wearPct) }}>
                                  {Math.round(c.wearPct)}%
                                </span>
                              </div>
                              <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                                <div className="h-full rounded-full" style={{ width: `${c.wearPct}%`, background: wearColor(c.wearPct) }} />
                              </div>
                              <div className="mt-0.5 text-[10px] text-slate-500">
                                {usd.format(c.actionCost)}
                              </div>
                            </div>
                            {/* Botón de etiqueta ESTABLE (no cambia de texto); se
                                deshabilita con check cuando el componente está al día. */}
                            {c.wearPct < 1 ? (
                              <span className="flex shrink-0 items-center gap-1 rounded-lg bg-emerald-500/10 px-2.5 py-1 text-[11px] font-semibold text-emerald-400">
                                <CheckCircle2 className="h-3.5 w-3.5" />
                                {t("economy.maint.ok")}
                              </span>
                            ) : (
                              <button
                                disabled={busy}
                                onClick={() => doService(c.id)}
                                className={`flex shrink-0 items-center gap-1 rounded-lg px-2.5 py-1 text-[11px] font-semibold transition-colors disabled:opacity-40 ${
                                  c.status === "due"
                                    ? "bg-red-500/20 text-red-300 hover:bg-red-500/30"
                                    : "bg-slate-800 text-slate-200 hover:bg-slate-700"
                                }`}
                              >
                                <Wrench className="h-3.5 w-3.5" />
                                {t("economy.maint.service")}
                              </button>
                            )}
                          </div>
                        ))}
                    </div>
                    <p className="mt-3 text-[11px] text-slate-500">{t("economy.maint.hint")}</p>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
