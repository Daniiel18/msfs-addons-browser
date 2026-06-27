import { useMemo } from "react";
import { motion } from "framer-motion";
import { Swords, X, Plane, Gauge, ArrowUp, Flag, Trophy } from "lucide-react";
import { t } from "../lib/i18n";
import {
  useLiveVsStore,
  vsChannelKey,
  loadVsSnapshot,
  type VsPilot,
  type VsPos,
  type VsLanding,
  type VsAircraft,
} from "../stores/useLiveVsStore";
import type { FlightLogEntry } from "../lib/types";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { AirlineLogo } from "./AirlineLogo";
import { icaoToName } from "../lib/airlineCodes";
import { useAircraftPhoto } from "./FlightBookView";

/** Distancia great-circle en millas náuticas (haversine). */
function distNm(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const R = 3440.065; // radio terrestre en NM
  const toRad = (d: number) => (d * Math.PI) / 180;
  const dLat = toRad(lat2 - lat1);
  const dLon = toRad(lon2 - lon1);
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.min(1, Math.sqrt(a)));
}

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
 * capitán; y telemetría en vivo (altitud, GS, distancia restante) con una
 * barra de carrera de progreso sobre la ruta. El rival ya se dibuja en el
 * routemap existente — esto es la ficha del duelo.
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
  const liveSelf = useLiveVsStore((s) => s.self);
  const liveRival = useLiveVsStore((s) => s.rival);
  const liveRivalPos = useLiveVsStore((s) => s.rivalPos);
  const liveSelfLanding = useLiveVsStore((s) => s.selfLanding);
  const liveRivalLanding = useLiveVsStore((s) => s.rivalLanding);
  const liveSelfAircraft = useLiveVsStore((s) => s.selfAircraft);
  const liveRivalAircraft = useLiveVsStore((s) => s.rivalAircraft);
  const currentChannel = useLiveVsStore((s) => s.currentChannel);
  const flightStatus = useFlightLogStore((s) => s.status);
  const ofp = useSimBriefStore((s) => s.flights)[0];

  // (v6.2.7) Canal del vuelo seleccionado (fecha+ruta). Si coincide con el duelo
  // EN VIVO, mostramos tiempo real; si no, cargamos el snapshot histórico de ESE
  // vuelo — así cada vuelo enseña su propio Crew VS.
  const entryChannel = useMemo(() => {
    if (!entry?.originIcao || !entry?.destinationIcao || !entry?.startedAt) {
      return null;
    }
    return vsChannelKey(
      entry.startedAt.slice(0, 10),
      entry.originIcao,
      entry.destinationIcao,
    );
  }, [entry?.originIcao, entry?.destinationIcao, entry?.startedAt]);

  const isLive =
    entryChannel != null &&
    entryChannel === currentChannel &&
    liveSelf != null &&
    liveRival != null;

  const snapshot = useMemo(
    () => (!isLive && entryChannel ? loadVsSnapshot(entryChannel) : null),
    [isLive, entryChannel],
  );

  const self = isLive ? liveSelf : snapshot?.self ?? null;
  const rival = isLive ? liveRival : snapshot?.rival ?? null;
  const selfLanding = isLive ? liveSelfLanding : snapshot?.selfLanding ?? null;
  const rivalLanding = isLive ? liveRivalLanding : snapshot?.rivalLanding ?? null;
  const selfAircraft = isLive ? liveSelfAircraft : snapshot?.selfAircraft ?? null;
  const rivalAircraft = isLive
    ? liveRivalAircraft
    : snapshot?.rivalAircraft ?? null;
  // Posiciones en vivo SÓLO en modo live (un vuelo pasado ya no se mueve).
  const rivalPos = isLive ? liveRivalPos : null;

  if (!self || !rival) return null;

  // Coords de la ruta (compartida — mismo OFP). Sirven para el progreso.
  const oLat = ofp?.originLat ?? null;
  const oLon = ofp?.originLon ?? null;
  const dLat = ofp?.destinationLat ?? null;
  const dLon = ofp?.destinationLon ?? null;
  const total =
    oLat != null && dLat != null ? distNm(oLat, oLon!, dLat, dLon!) : null;

  // Posición propia desde el watcher (sólo en vivo).
  const selfPos: VsPos | null =
    isLive &&
    flightStatus?.currentLat != null &&
    flightStatus.currentLon != null
      ? {
          lat: flightStatus.currentLat,
          lon: flightStatus.currentLon,
          heading: flightStatus.currentHeadingDeg ?? 0,
          alt: flightStatus.currentAltFt ?? 0,
          gs: flightStatus.currentGroundSpeedKt ?? 0,
        }
      : null;

  // Distancia restante a destino por piloto (menos = va ganando).
  const remaining = (p: VsPos | null): number | null =>
    p != null && dLat != null ? distNm(p.lat, p.lon, dLat, dLon!) : null;
  const selfRem = remaining(selfPos);
  const rivalRem = remaining(rivalPos);

  // Líder: el que tenga MENOS distancia restante.
  let leader: "self" | "rival" | null = null;
  if (selfRem != null && rivalRem != null) {
    leader = selfRem < rivalRem ? "self" : rivalRem < selfRem ? "rival" : null;
  }

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
        className="relative flex max-h-[90vh] w-full max-w-2xl flex-col overflow-y-auto rounded-2xl border border-fuchsia-500/30 bg-slate-950/95 shadow-2xl ring-1 ring-fuchsia-500/20"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-slate-800 bg-gradient-to-r from-sky-500/10 via-fuchsia-500/10 to-rose-500/10 px-4 py-3">
          <div className="flex items-center gap-2">
            <Swords className="h-4 w-4 text-fuchsia-300" />
            <span className="text-sm font-semibold text-slate-100">
              {t("vs.title")}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-slate-400 transition-colors hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Cuerpo: dos fichas + VS central */}
        <div className="grid grid-cols-[1fr_auto_1fr] items-stretch gap-0">
          <PilotCard
            pilot={self}
            pos={selfPos}
            remaining={selfRem}
            isLeader={leader === "self"}
            live={selfAircraft}
            isYou
            accent="sky"
          />
          <div className="flex flex-col items-center justify-center gap-1 bg-slate-900/40 px-2">
            <span className="text-lg font-black tracking-tighter text-fuchsia-300">
              VS
            </span>
          </div>
          <PilotCard
            pilot={rival}
            pos={rivalPos}
            remaining={rivalRem}
            isLeader={leader === "rival"}
            live={rivalAircraft}
            accent="rose"
          />
        </div>

        {/* Progreso del vuelo sobre la ruta. (v6.2.2) Se OCULTA cuando el vuelo
            propio ya terminó (aterrizaste) — no es una carrera, es un vuelo. */}
        {!selfLanding && (
          <div className="space-y-2 border-t border-slate-800 px-4 py-3">
            <div className="flex items-center justify-between text-[10px] uppercase tracking-wider text-slate-500">
              <span>{ofp?.originIcao ?? self.ofp.origin}</span>
              <span>{t("vs.progress")}</span>
              <span>{ofp?.destinationIcao ?? self.ofp.dest}</span>
            </div>
            <RaceBar
              label={self.name}
              remaining={selfRem}
              total={total}
              accent="sky"
              isYou
            />
            <RaceBar
              label={rival.name}
              remaining={rivalRem}
              total={total}
              accent="rose"
            />
            {total == null && (
              <p className="text-center text-[10px] text-slate-500">
                {t("vs.no_route")}
              </p>
            )}
          </div>
        )}

        {/* (v6.1 #34) Resultado del aterrizaje — aparece cuando alguno aterriza. */}
        {(selfLanding || rivalLanding) && (
          <LandingResult
            selfName={self.name}
            rivalName={rival.name}
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
    <div className="border-t border-slate-800 bg-slate-900/40 px-4 py-3">
      <div className="mb-2 flex items-center justify-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-fuchsia-300">
        <Trophy className="h-3.5 w-3.5" />
        {t("vs.touchdown")}
      </div>
      <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-2">
        <LandingCell name={selfName} r={self} win={winner === "self"} accent="sky" />
        <span className="text-xs font-black text-slate-500">—</span>
        <LandingCell name={rivalName} r={rival} win={winner === "rival"} accent="rose" />
      </div>
      {winner === "tie" && (
        <p className="mt-2 text-center text-[10px] text-slate-400">{t("vs.tie")}</p>
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
          : "border-slate-800 bg-slate-950/40"
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
            <span className="ml-1 text-xs font-semibold text-slate-400">fpm</span>
          </div>
          <div className="text-[10px] font-bold uppercase tracking-wide" style={{ color: g!.color }}>
            {t(g!.key)}
          </div>
        </>
      ) : (
        <div className="py-1.5 text-[11px] text-slate-500">{t("vs.waiting_landing")}</div>
      )}
    </div>
  );
}

function PilotCard({
  pilot,
  pos,
  remaining,
  isLeader,
  live,
  isYou = false,
  accent,
}: {
  pilot: VsPilot;
  pos: VsPos | null;
  remaining: number | null;
  isLeader: boolean;
  /** (v6.2.2) Avión REAL en vuelo (del sim). Manda sobre el del OFP. */
  live?: VsAircraft | null;
  isYou?: boolean;
  accent: "sky" | "rose";
}) {
  // (v6.2.2) Preferimos el avión REAL que vuela el piloto (matrícula del sim)
  // sobre el del OFP de SimBrief, que puede no coincidir.
  const reg = live?.registration || pilot.ofp.registration || null;
  const acType = live?.aircraftType || pilot.ofp.aircraft || null;
  const photo = useAircraftPhoto(reg);
  const alIcao = airlineIcao(pilot.ofp.callsign);
  const alName = live?.airlineName || icaoToName(alIcao);
  const ring = accent === "sky" ? "ring-sky-500/30" : "ring-rose-500/30";
  const text = accent === "sky" ? "text-sky-300" : "text-rose-300";

  return (
    <div className="flex flex-col gap-2 p-4">
      {/* Foto / silueta */}
      <div
        className={`relative flex aspect-[3/2] w-full items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-gradient-to-br from-slate-900 to-slate-950 ring-1 ${ring}`}
      >
        {photo ? (
          <img
            src={photo}
            alt={acType ?? ""}
            className="h-full w-full object-cover"
          />
        ) : (
          <Plane className="h-10 w-10 text-slate-700" />
        )}
        {isLeader && (
          <span className="absolute left-1.5 top-1.5 inline-flex items-center gap-1 rounded-full bg-amber-500/90 px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider text-amber-950">
            <Flag className="h-2.5 w-2.5" /> {t("vs.leading")}
          </span>
        )}
        {isYou && (
          <span className="absolute right-1.5 top-1.5 rounded-full bg-slate-950/80 px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider text-slate-200">
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
        <span className="truncate text-xs text-slate-300">
          {alName ?? pilot.ofp.callsign ?? "—"}
        </span>
      </div>

      {/* Modelo + matrícula */}
      <div className="flex items-center justify-between text-[11px] text-slate-400">
        <span className="font-mono text-slate-200">{acType ?? "—"}</span>
        <span className="font-mono uppercase">{reg ?? "—"}</span>
      </div>

      {/* Telemetría en vivo */}
      <div className="mt-1 grid grid-cols-3 gap-1 rounded-lg bg-slate-900/50 p-2 text-center">
        <Stat
          icon={<ArrowUp className="h-3 w-3" />}
          value={pos ? `${Math.round(pos.alt).toLocaleString()}` : "—"}
          unit="ft"
        />
        <Stat
          icon={<Gauge className="h-3 w-3" />}
          value={pos ? `${Math.round(pos.gs)}` : "—"}
          unit="kt"
        />
        <Stat
          icon={<Flag className="h-3 w-3" />}
          value={remaining != null ? `${Math.round(remaining)}` : "—"}
          unit="nm"
        />
      </div>
    </div>
  );
}

function Stat({
  icon,
  value,
  unit,
}: {
  icon: React.ReactNode;
  value: string;
  unit: string;
}) {
  return (
    <div className="flex flex-col items-center gap-0.5">
      <span className="text-slate-500">{icon}</span>
      <span className="font-mono text-sm font-semibold text-slate-100">
        {value}
      </span>
      <span className="text-[9px] uppercase tracking-wider text-slate-500">
        {unit}
      </span>
    </div>
  );
}

function RaceBar({
  label,
  remaining,
  total,
  accent,
  isYou = false,
}: {
  label: string;
  remaining: number | null;
  total: number | null;
  accent: "sky" | "rose";
  isYou?: boolean;
}) {
  const pct =
    remaining != null && total != null && total > 0
      ? Math.max(0, Math.min(100, ((total - remaining) / total) * 100))
      : 0;
  const bar = accent === "sky" ? "bg-sky-400" : "bg-rose-400";
  const dot = accent === "sky" ? "text-sky-300" : "text-rose-300";
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[10px]">
        <span className={`font-semibold ${dot}`}>
          {label}
          {isYou ? ` · ${t("vs.you")}` : ""}
        </span>
        <span className="text-slate-500">{Math.round(pct)}%</span>
      </div>
      <div className="relative h-2 overflow-hidden rounded-full bg-slate-800">
        <motion.div
          className={`absolute inset-y-0 left-0 ${bar}`}
          animate={{ width: `${pct}%` }}
          transition={{ duration: 0.5 }}
        />
      </div>
    </div>
  );
}
