import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import { Swords, X, Plane, Gauge, ArrowUp, Flag, Trophy, Route } from "lucide-react";
import { t } from "../lib/i18n";
import {
  findVsSnapshotEntry,
  fetchVsRivalMetrics,
  saveVsRivalMetrics,
  type VsPilot,
  type VsLanding,
  type VsAircraft,
  type VsMetrics,
} from "../stores/useLiveVsStore";
import type { FlightLogEntry } from "../lib/types";
import { AirlineLogo } from "./AirlineLogo";
import { icaoToName } from "../lib/airlineCodes";
import { useAircraftPhoto } from "./FlightBookView";

/** ICAO de aerolínea desde el callsign (primeras 3 letras). */
function airlineIcao(callsign: string | null | undefined): string | null {
  if (!callsign) return null;
  const m = callsign.toUpperCase().match(/^[A-Z]{3}/);
  return m ? m[0] : null;
}

/**
 * (v6 #3) Modal de combate Live VS — comparativa Daniel vs Héctor del MISMO
 * vuelo (misma ruta+día). Muestra para cada piloto: foto del avión
 * (planespotters por matrícula), aerolínea (logo + nombre), modelo, matrícula,
 * capitán; y (v8.3) las métricas del vuelo terminado (altitud y velocidad
 * máximas, distancia). El rival ya se dibuja en el routemap existente — esto
 * es la ficha del duelo.
 */
export function VsCombatModal({
  entry,
  onClose,
}: {
  /** (v6.2.7) Vuelo SELECCIONADO en el FlightBook. Si es el duelo en vivo se
   *  muestra en tiempo real; si es un vuelo pasado se carga su duelo guardado. */
  entry?: FlightLogEntry | null;
  onClose: () => void;
}) {
  // (v6.2.11) Crew VS = HISTÓRICO por vuelo. NO se muestra el duelo en vivo:
  // cada vuelo enseña SU propio duelo guardado, cargado por el canal del vuelo
  // seleccionado (fecha de inicio + ruta). El snapshot sólo existe si Héctor
  // también voló ese vuelo (guarda a ambos pilotos), así que el modal sólo se
  // abre para duelos reales y completos.
  const found = useMemo(() => {
    if (!entry?.originIcao || !entry?.destinationIcao || !entry?.startedAt) {
      return null;
    }
    return findVsSnapshotEntry(
      entry.startedAt.slice(0, 10),
      entry.originIcao,
      entry.destinationIcao,
    );
  }, [entry?.originIcao, entry?.destinationIcao, entry?.startedAt]);
  const snapshot = found?.snapshot ?? null;

  // (v8.3) Métricas del vuelo terminado: las mías salen del vuelo del
  // FlightBook; las del rival, de su fila en `vs_results` (las sube su SimFleet
  // al cerrar el vuelo). Se muestra la caché al instante y se refresca.
  const [rivalMetrics, setRivalMetrics] = useState<VsMetrics | null>(
    snapshot?.rivalMetrics ?? null,
  );
  useEffect(() => {
    setRivalMetrics(found?.snapshot.rivalMetrics ?? null);
    if (!found) return;
    let alive = true;
    void fetchVsRivalMetrics(
      found.channel,
      found.snapshot.self?.identity ?? null,
      found.snapshot.rival?.identity ?? null,
    ).then((m) => {
      if (!alive || !m) return;
      setRivalMetrics(m);
      saveVsRivalMetrics(found.channel, m);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [found?.channel]);
  const selfMetrics: VsMetrics | null = entry
    ? {
        maxAltFt: entry.maxAltitudeFt ?? null,
        maxGsKt: entry.maxGroundSpeedKt ?? null,
        distanceNm: entry.distanceNm ?? null,
      }
    : null;

  const self = snapshot?.self ?? null;
  const rival = snapshot?.rival ?? null;
  const selfLanding = snapshot?.selfLanding ?? null;
  const rivalLanding = snapshot?.rivalLanding ?? null;
  const selfAircraft = snapshot?.selfAircraft ?? null;
  const rivalAircraft = snapshot?.rivalAircraft ?? null;
  // Histórico: sin carrera en vivo (el vuelo ya terminó) → nadie va "líder".
  const leader: "self" | "rival" | null = null;

  // (v6.2.13) Basta con que haya datos de UN lado para mostrar el duelo: si el
  // rival no voló (o aún no llegó su resultado), su ficha sale como "esperando".
  if (!self && !rival) return null;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="absolute inset-0 z-30 flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <motion.div
        initial={{ scale: 0.94, y: 12 }}
        animate={{ scale: 1, y: 0 }}
        exit={{ scale: 0.94, y: 12 }}
        onClick={(e) => e.stopPropagation()}
        className="relative flex max-h-[90vh] w-full max-w-2xl flex-col overflow-y-auto rounded-2xl border border-fuchsia-500/30 bg-sf-bg/95 shadow-2xl ring-1 ring-fuchsia-500/20"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-sf-border bg-gradient-to-r from-sky-500/10 via-fuchsia-500/10 to-rose-500/10 px-4 py-3">
          <div className="flex items-center gap-2">
            <Swords className="h-4 w-4 text-fuchsia-300" />
            <span className="text-sm font-semibold text-sf-text">
              {t("vs.title")}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-sf-muted transition-colors hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Cuerpo: dos fichas + VS central */}
        <div className="grid grid-cols-[1fr_auto_1fr] items-stretch gap-0">
          <PilotCard
            pilot={self}
            metrics={selfMetrics}
            isLeader={leader === "self"}
            live={selfAircraft}
            isYou
            accent="sky"
          />
          <div className="flex flex-col items-center justify-center gap-1 bg-sf-surface px-2">
            <span className="text-lg font-black tracking-tighter text-fuchsia-300">
              VS
            </span>
          </div>
          <PilotCard
            pilot={rival}
            metrics={rivalMetrics}
            isLeader={leader === "rival"}
            live={rivalAircraft}
            accent="rose"
          />
        </div>

        {/* (v6.2.11) Crew VS histórico: sin barra de progreso en vivo (el vuelo
            ya terminó). Sólo fichas + resultado de aterrizaje. */}

        {/* (v6.1 #34) Resultado del aterrizaje — aparece cuando alguno aterriza. */}
        {(selfLanding || rivalLanding) && (
          <LandingResult
            selfName={self?.name ?? t("vs.you")}
            rivalName={rival?.name ?? "—"}
            self={selfLanding}
            rival={rivalLanding}
          />
        )}
      </motion.div>
    </motion.div>
  );
}

const GRADE_META: Record<string, { color: string; key: string }> = {
  butter: { color: "#3fbf78", key: "osd.butter" },
  acceptable: { color: "#f59e0b", key: "osd.acceptable" },
  hard: { color: "#f43f5e", key: "osd.hard" },
};

function LandingResult({
  selfName,
  rivalName,
  self,
  rival,
}: {
  selfName: string;
  rivalName: string;
  self: VsLanding | null;
  rival: VsLanding | null;
}) {
  // Mejor aterrizaje = FPM más cercano a 0 (menos negativo).
  let winner: "self" | "rival" | "tie" | null = null;
  if (self && rival) {
    winner = self.fpm > rival.fpm ? "self" : rival.fpm > self.fpm ? "rival" : "tie";
  }
  return (
    <div className="border-t border-sf-border bg-sf-surface px-4 py-3">
      <div className="mb-2 flex items-center justify-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-fuchsia-300">
        <Trophy className="h-3.5 w-3.5" />
        {t("vs.touchdown")}
      </div>
      <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-2">
        <LandingCell name={selfName} r={self} win={winner === "self"} accent="sky" />
        <span className="text-xs font-black text-sf-muted">—</span>
        <LandingCell name={rivalName} r={rival} win={winner === "rival"} accent="rose" />
      </div>
      {winner === "tie" && (
        <p className="mt-2 text-center text-[11px] text-sf-muted">{t("vs.tie")}</p>
      )}
    </div>
  );
}

function LandingCell({
  name,
  r,
  win,
  accent,
}: {
  name: string;
  r: VsLanding | null;
  win: boolean;
  accent: "sky" | "rose";
}) {
  const text = accent === "sky" ? "text-sky-300" : "text-rose-300";
  const g = r ? (GRADE_META[r.grade] ?? GRADE_META.acceptable) : null;
  return (
    <div
      className={`rounded-xl border p-2 text-center ${
        win
          ? "border-amber-500/50 bg-amber-500/10"
          : "border-sf-border bg-sf-bg/40"
      }`}
    >
      <div className={`flex items-center justify-center gap-1 truncate text-[11px] font-semibold ${text}`}>
        {win && <Trophy className="h-3 w-3 text-amber-400" />}
        {name}
      </div>
      {r ? (
        <>
          <div className="text-xl font-extrabold" style={{ color: g!.color }}>
            {Math.round(r.fpm)}
            <span className="ml-1 text-xs font-semibold text-sf-muted">fpm</span>
          </div>
          <div className="text-[11px] font-bold uppercase tracking-wide" style={{ color: g!.color }}>
            {t(g!.key)}
          </div>
        </>
      ) : (
        <div className="py-1.5 text-[11px] text-sf-muted">{t("vs.waiting_landing")}</div>
      )}
    </div>
  );
}

function PilotCard({
  pilot,
  metrics,
  isLeader,
  live,
  isYou = false,
  accent,
}: {
  pilot: VsPilot | null;
  /** (v8.3) Métricas del vuelo terminado (null → "—"). */
  metrics: VsMetrics | null;
  isLeader: boolean;
  /** (v6.2.2) Avión REAL en vuelo (del sim). Manda sobre el del OFP. */
  live?: VsAircraft | null;
  isYou?: boolean;
  accent: "sky" | "rose";
}) {
  // (v6.2.2) Preferimos el avión REAL que vuela el piloto (matrícula del sim)
  // sobre el del OFP de SimBrief, que puede no coincidir.
  const reg = live?.registration || pilot?.ofp.registration || null;
  const acType = live?.aircraftType || pilot?.ofp.aircraft || null;
  const photo = useAircraftPhoto(reg);
  const alIcao = airlineIcao(pilot?.ofp.callsign);
  const alName = live?.airlineName || icaoToName(alIcao);
  const ring = accent === "sky" ? "ring-sky-500/30" : "ring-rose-500/30";
  const text = accent === "sky" ? "text-sky-300" : "text-rose-300";

  // (v6.2.13) Sin datos de este piloto (no voló / aún no llega su resultado).
  if (!pilot) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 p-4 text-center">
        <div
          className={`flex aspect-[3/2] w-full items-center justify-center rounded-xl border border-dashed border-sf-border bg-sf-bg/40 ring-1 ${ring}`}
        >
          <Plane className="h-10 w-10 text-sf-muted/70" />
        </div>
        <span className={`text-sm font-bold ${text}`}>
          {isYou ? t("vs.you") : "—"}
        </span>
        <span className="text-[11px] text-sf-muted">
          {t("vs.waiting_landing")}
        </span>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2 p-4">
      {/* Foto / silueta */}
      <div
        className={`relative flex aspect-[3/2] w-full items-center justify-center overflow-hidden rounded-xl border border-sf-border bg-gradient-to-br from-slate-900 to-slate-950 ring-1 ${ring}`}
      >
        {photo ? (
          <img
            src={photo}
            alt={acType ?? ""}
            className="h-full w-full object-cover"
          />
        ) : (
          <Plane className="h-10 w-10 text-sf-muted/70" />
        )}
        {isLeader && (
          <span className="absolute left-1.5 top-1.5 inline-flex items-center gap-1 rounded-full bg-amber-500/90 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wider text-amber-950">
            <Flag className="h-2.5 w-2.5" /> {t("vs.leading")}
          </span>
        )}
        {isYou && (
          <span className="absolute right-1.5 top-1.5 rounded-full bg-slate-950/80 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wider text-sf-text">
            {t("vs.you")}
          </span>
        )}
      </div>

      {/* Capitán */}
      <div className="flex items-center justify-between">
        <span className={`truncate text-sm font-bold ${text}`}>{pilot.name}</span>
      </div>

      {/* Aerolínea */}
      <div className="flex items-center gap-2">
        <AirlineLogo icao={alIcao} name={alName} size={18} />
        <span className="truncate text-xs text-sf-text/90">
          {alName ?? pilot.ofp.callsign ?? "—"}
        </span>
      </div>

      {/* Modelo + matrícula */}
      <div className="flex items-center justify-between text-[11px] text-sf-muted">
        <span className="font-mono text-sf-text">{acType ?? "—"}</span>
        <span className="font-mono uppercase">{reg ?? "—"}</span>
      </div>

      {/* (v8.3) Métricas del vuelo terminado */}
      <div className="mt-1 grid grid-cols-3 gap-1 rounded-lg bg-sf-surface p-2 text-center">
        <Stat
          icon={<ArrowUp className="h-3 w-3" />}
          value={fmtMetric(metrics?.maxAltFt)}
          unit={t("vs.stat.max_alt")}
          title={t("vs.stat.max_alt_title")}
        />
        <Stat
          icon={<Gauge className="h-3 w-3" />}
          value={fmtMetric(metrics?.maxGsKt)}
          unit={t("vs.stat.max_gs")}
          title={t("vs.stat.max_gs_title")}
        />
        <Stat
          icon={<Route className="h-3 w-3" />}
          value={fmtMetric(metrics?.distanceNm)}
          unit={t("vs.stat.distance")}
          title={t("vs.stat.distance_title")}
        />
      </div>
    </div>
  );
}

function fmtMetric(v: number | null | undefined): string {
  return v != null ? Math.round(v).toLocaleString() : "—";
}

function Stat({
  icon,
  value,
  unit,
  title,
}: {
  icon: React.ReactNode;
  value: string;
  unit: string;
  title?: string;
}) {
  return (
    <div className="flex flex-col items-center gap-0.5" title={title}>
      <span className="text-sf-muted">{icon}</span>
      <span className="font-mono text-sm font-semibold text-sf-text">
        {value}
      </span>
      <span className="text-[11px] uppercase tracking-wider text-sf-muted">
        {unit}
      </span>
    </div>
  );
}

