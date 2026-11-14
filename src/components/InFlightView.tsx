import { useEffect, useMemo, useRef, useState, type MutableRefObject, type ReactNode } from "react";
import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  BookOpen,
  CheckCircle2,
  ChevronDown,
  ClipboardCheck,
  Crosshair,
  Maximize2,
  MapPin,
  PlayCircle,
  Plane,
  PlaneLanding,
  RefreshCw,
  TriangleAlert,
  X,
} from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { api } from "../lib/tauri";
import { useUnits } from "../lib/units";
import { usePreflight } from "../lib/usePreflight";
import { useLatestOfp } from "../lib/useLatestOfp";
import { gcNm, greatCircle, unwrap, type Pt } from "../lib/geo";
import type { PreflightCheck } from "../lib/preflight";
import type { AirportBrief, FlightLogEntry, SimBriefFlight } from "../lib/types";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { usePreflightStore } from "../stores/usePreflightStore";
import { useAppStore } from "../stores/useAppStore";
import { useToastStore } from "../stores/useToastStore";
import { friendFlying, useCrewLiveStore } from "../stores/useCrewLiveStore";
import { useLiveVsStore } from "../stores/useLiveVsStore";
import { clipForFlight, useLandingClipsStore } from "../stores/useLandingClipsStore";
import { DARK_STYLE } from "./MapView";
import { effectiveRoute, phaseDisplay } from "./FlyingNowBadge";
import { airportPhotoUrl } from "./AirportCard";
import { cleanAircraftLabel, useAircraftPhoto } from "./FlightBookView";
import { ClipPlayerModal } from "./ClipPlayerModal";
import { LaunchButton } from "./LaunchButton";
import { DispatchSelector, liveryNote } from "./DispatchSelector";
import { useDispatchPlan, useDispatchStore } from "../stores/useDispatchStore";
import { dispatchFlight, sceneriesFor } from "../lib/dispatch";
import { useSimContentStore } from "../stores/useSimContentStore";

/**
 * (8.4.x) Pantalla «En vuelo» (diseño inspirado en el Flight Preparation de
 * Bob Dispatcher): mapa oscuro arriba con la ruta del plan, tu avión en vivo
 * con su traza y el del otro piloto (Héctor) en fucsia; abajo una tarjeta con
 * el vuelo, lo que necesita (escenarios, GSX, avión) y su estado (antes, en
 * vuelo o al aterrizar). Lo "en vivo" vive aquí; el FlightBook queda como
 * historial.
 */


const PLANE_PATH =
  "M21 16v-2l-8-5V3.5C13 2.67 12.33 2 11.5 2S10 2.67 10 3.5V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z";
const OWN_COLOR = "#fbbf24";
const FRIEND_COLOR = "#e879f9";
const PLAN_COLOR = "#35cdb2";

function planeElement(color: string, title: string): HTMLDivElement {
  const el = document.createElement("div");
  el.style.width = "30px";
  el.style.height = "30px";
  el.style.cursor = "pointer";
  el.title = title;
  el.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="30" height="30"><path fill="${color}" stroke="#0e0e10" stroke-width="0.8" stroke-linejoin="round" d="${PLANE_PATH}"/></svg>`;
  return el;
}

interface MapAirport {
  icao: string;
  lon: number;
  lat: number;
}

interface MapProps {
  origin: MapAirport | null;
  dest: MapAirport | null;
  fixes: Pt[];
  own: { lon: number; lat: number; heading: number } | null;
  ownTrack: Pt[];
  friend: { lon: number; lat: number; heading: number; name: string } | null;
  friendTrack: Pt[];
  follow: boolean;
  fitSignal: number;
  focusFriend: number;
  /** Alto que tapa la tarjeta de abajo (para encuadrar por encima). */
  bottomPad: number;
  onUserMove: () => void;
}

const EMPTY_FC = { type: "FeatureCollection" as const, features: [] };

function line(coords: Pt[]): GeoJSON.Feature {
  return { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: coords } };
}

/** Fuente GeoJSON del mapa si ya existe (tras recrear el mapa en una recarga
 *  en caliente, los efectos pueden correr antes de su `load`). */
function src(map: maplibregl.Map, id: string): maplibregl.GeoJSONSource | null {
  return (map.getSource(id) as maplibregl.GeoJSONSource | undefined) ?? null;
}

function InFlightMap(p: MapProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [ready, setReady] = useState(false);
  const ownMarker = useRef<maplibregl.Marker | null>(null);
  const friendMarker = useRef<maplibregl.Marker | null>(null);
  const fittedFor = useRef("");
  /** El usuario movió el mapa: no se re-encuadra solo. */
  const userMoved = useRef(false);
  const fittedPad = useRef(0);

  useEffect(() => {
    if (!ref.current) return;
    const style = JSON.parse(JSON.stringify(DARK_STYLE)) as maplibregl.StyleSpecification;
    // Más oscuro y sin color, como el mapa de Bob: la ruta y los aviones resaltan.
    for (const l of style.layers) {
      if (l.type === "raster") {
        (l as maplibregl.RasterLayerSpecification).paint = { "raster-brightness-max": 0.62, "raster-saturation": -1 };
      }
    }
    const map = new maplibregl.Map({
      container: ref.current,
      style,
      center: [0, 25],
      zoom: 1.4,
      // Arriba a la derecha (como en Bob): abajo la tapa la tarjeta.
      attributionControl: false,
      dragRotate: false,
    });
    map.addControl(new maplibregl.AttributionControl({ compact: true }), "top-right");
    mapRef.current = map;
    map.on("load", () => {
      map.addSource("if-plan", { type: "geojson", data: EMPTY_FC });
      map.addSource("if-own", { type: "geojson", data: EMPTY_FC });
      map.addSource("if-friend", { type: "geojson", data: EMPTY_FC });
      map.addSource("if-apt", { type: "geojson", data: EMPTY_FC });
      map.addLayer({ id: "if-plan", type: "line", source: "if-plan", paint: { "line-color": PLAN_COLOR, "line-width": 2.2, "line-opacity": 0.85 } });
      map.addLayer({ id: "if-friend", type: "line", source: "if-friend", paint: { "line-color": FRIEND_COLOR, "line-width": 2.4, "line-opacity": 0.9 } });
      map.addLayer({ id: "if-own", type: "line", source: "if-own", paint: { "line-color": OWN_COLOR, "line-width": 2.8 } });
      map.addLayer({
        id: "if-apt",
        type: "circle",
        source: "if-apt",
        paint: { "circle-radius": 6, "circle-color": "#1c1c1e", "circle-stroke-color": "#f5f5f7", "circle-stroke-width": 2.2 },
      });
      map.addLayer({
        id: "if-apt-label",
        type: "symbol",
        source: "if-apt",
        layout: {
          "text-field": ["get", "icao"],
          "text-font": ["Open Sans Semibold"],
          "text-size": 12,
          "text-offset": [0, 1.25],
          "text-anchor": "top",
        },
        paint: { "text-color": "#f5f5f7", "text-halo-color": "#0e0e10", "text-halo-width": 1.4 },
      });
      setReady(true);
    });
    for (const ev of ["dragstart", "wheel", "touchstart"] as const) {
      map.on(ev, () => {
        userMoved.current = true;
        p.onUserMove();
      });
    }
    return () => {
      map.remove();
      mapRef.current = null;
      setReady(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const planCoords = useMemo(() => {
    const pts: Pt[] = [];
    if (p.origin) pts.push([p.origin.lon, p.origin.lat]);
    if (p.fixes.length > 0) pts.push(...p.fixes);
    else if (p.origin && p.dest) pts.push(...greatCircle([p.origin.lon, p.origin.lat], [p.dest.lon, p.dest.lat]).slice(1, -1));
    if (p.dest) pts.push([p.dest.lon, p.dest.lat]);
    return unwrap(pts);
  }, [p.origin, p.dest, p.fixes]);

  // Ruta del plan y aeropuertos.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !ready) return;
    const plan = src(map, "if-plan");
    const aptSrc = src(map, "if-apt");
    if (!plan || !aptSrc) return;
    plan.setData({
      type: "FeatureCollection",
      features: planCoords.length >= 2 ? [line(planCoords)] : [],
    });
    const apts = [p.origin, p.dest].filter((a): a is MapAirport => !!a);
    const lon0 = planCoords[0]?.[0];
    aptSrc.setData({
      type: "FeatureCollection",
      features: apts.map((a, i) => ({
        type: "Feature",
        properties: { icao: a.icao },
        geometry: {
          type: "Point",
          // El destino con la misma longitud "desenrollada" que la línea.
          coordinates: [i === 1 && planCoords.length ? planCoords[planCoords.length - 1][0] : (lon0 ?? a.lon), a.lat],
        },
      })),
    });
  }, [ready, planCoords, p.origin, p.dest]);

  // Trazas.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !ready) return;
    const ownSrc = src(map, "if-own");
    const friendSrc = src(map, "if-friend");
    if (!ownSrc || !friendSrc) return;
    ownSrc.setData({
      type: "FeatureCollection",
      features: p.ownTrack.length >= 2 ? [line(unwrap(p.ownTrack))] : [],
    });
    friendSrc.setData({
      type: "FeatureCollection",
      features: p.friendTrack.length >= 2 ? [line(unwrap(p.friendTrack))] : [],
    });
  }, [ready, p.ownTrack, p.friendTrack]);

  // Aviones.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !ready) return;
    const sync = (
      ref: MutableRefObject<maplibregl.Marker | null>,
      pos: { lon: number; lat: number; heading: number } | null,
      color: string,
      title: string,
    ) => {
      if (!pos) {
        ref.current?.remove();
        ref.current = null;
        return;
      }
      if (!ref.current) {
        ref.current = new maplibregl.Marker({ element: planeElement(color, title), rotationAlignment: "map" })
          .setLngLat([pos.lon, pos.lat])
          .addTo(map);
      } else {
        ref.current.setLngLat([pos.lon, pos.lat]);
      }
      ref.current.setRotation(pos.heading);
    };
    sync(ownMarker, p.own, OWN_COLOR, t("inflight.map.you"));
    sync(friendMarker, p.friend, FRIEND_COLOR, p.friend?.name ?? "");
  }, [ready, p.own, p.friend]);

  // Encuadre: la ruta entera (y los aviones) por encima de la tarjeta.
  const fit = () => {
    const map = mapRef.current;
    if (!map) return;
    const pts: Pt[] = [...planCoords];
    if (p.own) pts.push([p.own.lon, p.own.lat]);
    if (pts.length === 0 && p.friend) pts.push([p.friend.lon, p.friend.lat]);
    if (pts.length === 0) return;
    const b = new maplibregl.LngLatBounds(pts[0], pts[0]);
    for (const q of pts) b.extend(q);
    fittedPad.current = p.bottomPad;
    map.fitBounds(b, {
      padding: { top: 70, left: 60, right: 60, bottom: p.bottomPad + 40 },
      maxZoom: pts.length === 1 ? 8 : 9,
      duration: 700,
    });
  };
  const fitKey = `${p.origin?.icao ?? ""}-${p.dest?.icao ?? ""}-${planCoords.length > 0}-${p.own ? 1 : 0}`;
  useEffect(() => {
    if (!ready || fittedFor.current === fitKey) return;
    fittedFor.current = fitKey;
    userMoved.current = false;
    fit();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, fitKey]);
  useEffect(() => {
    if (ready && p.fitSignal > 0) {
      userMoved.current = false;
      fit();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.fitSignal]);
  // La tarjeta creció o encogió (p. ej. al llegar el plan): se re-encuadra
  // para que la ruta no quede debajo, salvo que el usuario ya movió el mapa.
  useEffect(() => {
    if (!ready || userMoved.current || p.follow) return;
    if (Math.abs(p.bottomPad - fittedPad.current) < 40) return;
    fit();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, p.bottomPad]);

  // Seguir mi avión / ir al del otro piloto.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !ready || !p.follow || !p.own) return;
    map.easeTo({ center: [p.own.lon, p.own.lat], padding: { top: 0, left: 0, right: 0, bottom: p.bottomPad }, duration: 500 });
  }, [ready, p.follow, p.own, p.bottomPad]);
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !ready || p.focusFriend === 0 || !p.friend) return;
    map.easeTo({
      center: [p.friend.lon, p.friend.lat],
      zoom: Math.max(map.getZoom(), 6),
      padding: { top: 0, left: 0, right: 0, bottom: p.bottomPad },
      duration: 700,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.focusFriend]);

  return <div ref={ref} className="absolute inset-0" />;
}

// ───────────────────────────── piezas de la tarjeta ─────────────────────────────

function Column({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("min-w-0 px-3 py-2.5", className)}>
      <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{title}</div>
      {children}
    </div>
  );
}

function Fact({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="min-w-0">
      <div className="text-[10px] text-sf-muted">{label}</div>
      <div className={cn("truncate text-xs font-semibold text-sf-text", mono && "font-mono")}>{value}</div>
    </div>
  );
}

type Tone = "ok" | "warn" | "bad" | "off";

const TONE_TEXT: Record<Tone, string> = {
  ok: "text-emerald-300",
  warn: "text-amber-300",
  bad: "text-rose-300",
  off: "text-sf-muted",
};
const TONE_DOT: Record<Tone, string> = {
  ok: "bg-emerald-400",
  warn: "bg-amber-400",
  bad: "bg-rose-400",
  off: "border border-sf-muted bg-transparent",
};

function StatusLine({ tone, label }: { tone: Tone; label: string }) {
  return <span className={cn("block truncate text-right text-xs font-medium", TONE_TEXT[tone])}>{label}</span>;
}

function DotLabel({ tone, label }: { tone: Tone; label: string }) {
  return (
    <span className="flex items-center justify-end gap-1.5 text-[11px] text-sf-muted">
      {label}
      <span className={cn("h-2 w-2 shrink-0 rounded-full", TONE_DOT[tone])} />
    </span>
  );
}

function NeedRow({
  thumb,
  fallback,
  title,
  sub,
  right,
  onClick,
}: {
  thumb: string | null;
  fallback: ReactNode;
  title: string;
  sub: string;
  right: ReactNode;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 rounded-lg p-1 text-left transition-colors hover:bg-sf-raised/60"
    >
      <span className="flex h-9 w-13 shrink-0 items-center justify-center overflow-hidden rounded-md bg-sf-raised ring-1 ring-sf-border">
        {thumb ? <img src={thumb} alt="" className="h-full w-full object-cover" /> : fallback}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-xs font-semibold text-sf-text">{title}</span>
        <span className="block truncate text-[10px] text-sf-muted">{sub}</span>
      </span>
      <span className="w-24 shrink-0 space-y-0.5">{right}</span>
    </button>
  );
}

function useAirportPhoto(icao: string | null, name: string | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setUrl(null);
    if (!icao) return;
    void airportPhotoUrl(icao, name).then((u) => {
      if (alive) setUrl(u);
    });
    return () => {
      alive = false;
    };
  }, [icao, name]);
  return url;
}

/** Estado del escenario de una pata según el pre-vuelo. */
function sceneryState(checks: PreflightCheck[], leg: "dep" | "arr"): { tone: Tone; key: string } | null {
  const c = checks.find((x) => x.id === `${leg}_scn`);
  if (!c) return null;
  if (c.titleKey.endsWith("_ok")) return { tone: "ok", key: "inflight.need.installed" };
  if (c.titleKey.endsWith("_bypass")) return { tone: "off", key: "inflight.need.default_ok" };
  if (c.titleKey.endsWith("_sim")) return { tone: "off", key: "dispatch.status.sim" };
  if (c.titleKey.endsWith("_disabled")) return { tone: "warn", key: "inflight.need.disabled" };
  if (c.titleKey.endsWith("_update")) return { tone: "warn", key: "inflight.need.update" };
  return { tone: "warn", key: "inflight.need.missing" };
}

function fmtClock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function fmtEte(s: number | null | undefined): string {
  if (!s || s <= 0) return "—";
  const h = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  return h > 0 ? `${h} h ${String(m).padStart(2, "0")} min` : `${m} min`;
}

function fpmTone(fpm: number): string {
  return fpm > -100 ? "text-cyan-300" : fpm > -300 ? "text-emerald-300" : fpm > -400 ? "text-amber-300" : "text-rose-400";
}

// ───────────────────────────── pantalla ─────────────────────────────

export function InFlightView() {
  const u = useUnits();
  const status = useFlightLogStore((s) => s.status);
  const entries = useFlightLogStore((s) => s.entries);
  const ofps = useSimBriefStore((s) => s.flights);
  const latestOfp = useLatestOfp();
  const packages = useCommunityStore((s) => s.packages);
  const gsxIcaos = useGsxLocalStore((s) => s.installedIcaos);
  const { result: pre } = usePreflight();
  const openPreflight = usePreflightStore((s) => s.setOpen);
  const push = useToastStore((s) => s.push);

  // ── Vivo / plan / vuelo hecho ──
  const live = !!status?.simconnectConnected && status.currentLat != null && status.currentLon != null;
  const liveEntry = live ? (entries.find((e) => e.endedAt === null) ?? null) : null;
  const liveRoute = live && status ? effectiveRoute(status, ofps) : null;
  const ofp: SimBriefFlight | null = (live ? liveRoute?.matchedOfp : null) ?? latestOfp;
  const lastDone = useMemo(
    () =>
      entries
        .filter((e) => e.endedAt != null && e.landingFpm != null)
        .sort((a, b) => (b.endedAt ?? "").localeCompare(a.endedAt ?? ""))[0] ?? null,
    [entries],
  );
  // Vuelo ya hecho: el último terminado es el de este plan (o no hay plan y
  // terminó hace poco). Se ve su resultado hasta que llegue otro OFP.
  const doneEntry: FlightLogEntry | null = useMemo(() => {
    if (live || !lastDone?.endedAt) return null;
    const ended = Date.parse(lastDone.endedAt);
    if (!latestOfp) return Date.now() - ended < 6 * 3600_000 ? lastDone : null;
    const ofpMs = latestOfp.generatedAt ? parseInt(latestOfp.generatedAt, 10) * 1000 : 0;
    const same =
      (lastDone.originIcao ?? "").toUpperCase() === latestOfp.originIcao.toUpperCase() &&
      (lastDone.destinationIcao ?? "").toUpperCase() === latestOfp.destinationIcao.toUpperCase();
    return same && Date.parse(lastDone.startedAt) >= ofpMs - 3600_000 ? lastDone : null;
  }, [live, lastDone, latestOfp]);
  const landedLive = !!liveEntry && liveEntry.landingFpm != null;

  const originIcao = (live ? liveRoute?.effectiveOrigin : null) ?? doneEntry?.originIcao ?? ofp?.originIcao ?? null;
  const destIcao = (live ? liveRoute?.effectiveDestination : null) ?? doneEntry?.destinationIcao ?? ofp?.destinationIcao ?? null;
  const planMatches = !!ofp && ofp.originIcao === originIcao && ofp.destinationIcao === destIcao;

  // Nombres / ciudades de los aeropuertos.
  const [airports, setAirports] = useState<Record<string, AirportBrief>>({});
  useEffect(() => {
    const want = [originIcao, destIcao].filter((x): x is string => !!x && !airports[x]);
    if (want.length === 0) return;
    void api
      .lookupAirports(want)
      .then((list) => setAirports((prev) => ({ ...prev, ...Object.fromEntries(list.map((a) => [a.icao, a])) })))
      .catch(() => {});
  }, [originIcao, destIcao, airports]);
  const city = (icao: string | null, fallback: string | null | undefined) =>
    (icao && (airports[icao]?.municipality || airports[icao]?.name)) || fallback || "";

  // Coordenadas para el mapa.
  const originPt: MapAirport | null =
    planMatches && ofp ? { icao: ofp.originIcao, lon: ofp.originLon, lat: ofp.originLat } : doneEntry && doneEntry.originIcao
      ? { icao: doneEntry.originIcao, lon: doneEntry.originLon, lat: doneEntry.originLat }
      : null;
  const destPt: MapAirport | null =
    planMatches && ofp
      ? { icao: ofp.destinationIcao, lon: ofp.destinationLon, lat: ofp.destinationLat }
      : doneEntry?.destinationIcao && doneEntry.destinationLat != null && doneEntry.destinationLon != null
        ? { icao: doneEntry.destinationIcao, lon: doneEntry.destinationLon, lat: doneEntry.destinationLat }
        : null;
  const fixes: Pt[] = useMemo(
    () => (planMatches && ofp?.routeFixes?.length ? ofp.routeFixes.map((f) => [f.lon, f.lat] as Pt) : []),
    [planMatches, ofp],
  );

  // Traza propia: la del vuelo abierto (o la del vuelo hecho), cada 10 s.
  const trackFor = liveEntry?.id ?? doneEntry?.id ?? null;
  const [track, setTrack] = useState<Pt[]>([]);
  useEffect(() => {
    let alive = true;
    setTrack([]);
    if (trackFor == null) return;
    const load = () =>
      void api
        .getFlightTrack(trackFor)
        .then((pts) => {
          if (alive) setTrack(pts.map((q) => [q.lon, q.lat] as Pt));
        })
        .catch(() => {});
    load();
    const id = liveEntry ? window.setInterval(load, 10_000) : null;
    return () => {
      alive = false;
      if (id != null) window.clearInterval(id);
    };
  }, [trackFor, liveEntry]);
  const ownLon = live ? status?.currentLon ?? null : null;
  const ownLat = live ? status?.currentLat ?? null : null;
  const ownHdg = status?.currentHeadingDeg ?? 0;
  const own = useMemo(
    () => (ownLon != null && ownLat != null ? { lon: ownLon, lat: ownLat, heading: ownHdg } : null),
    [ownLon, ownLat, ownHdg],
  );
  const ownTrack = useMemo(() => (own && liveEntry ? [...track, [own.lon, own.lat] as Pt] : track), [track, own, liveEntry]);

  // ── El otro piloto (presencia siempre activa; si no, el del duelo) ──
  const friend = useCrewLiveStore((s) => s.friend);
  const friendFlight = useCrewLiveStore((s) => s.friendFlight);
  const friendPos = useCrewLiveStore((s) => s.friendPos);
  const friendPosAt = useCrewLiveStore((s) => s.friendPosAt);
  const friendPhase = useCrewLiveStore((s) => s.friendPhase);
  const friendTrackRaw = useCrewLiveStore((s) => s.friendTrack);
  const rival = useLiveVsStore((s) => s.rival);
  const rivalPos = useLiveVsStore((s) => s.rivalPos);
  const rivalPosAt = useLiveVsStore((s) => s.rivalPosAt);
  const rivalPhase = useLiveVsStore((s) => s.rivalPhase);
  const rivalTrack = useLiveVsStore((s) => s.rivalTrack);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 5000);
    return () => window.clearInterval(id);
  }, []);
  const viaPresence = !!friend && !!friendPos && friendFlying(friendPosAt, now);
  const viaDuel = !viaPresence && !!rival && !!rivalPos && friendFlying(rivalPosAt, now);
  const mate = viaPresence
    ? {
        name: friend!.name,
        pos: friendPos!,
        phase: friendPhase,
        origin: friendFlight?.origin ?? null,
        dest: friendFlight?.dest ?? null,
        aircraft: friendFlight?.aircraft ?? null,
        track: friendTrackRaw,
      }
    : viaDuel
      ? {
          name: rival!.name,
          pos: rivalPos!,
          phase: rivalPhase,
          origin: rival!.ofp.origin,
          dest: rival!.ofp.dest,
          aircraft: rival!.ofp.aircraft,
          track: rivalTrack,
        }
      : null;
  const mateTrackSrc = mate?.track;
  const mateTrack: Pt[] = useMemo(() => (mateTrackSrc ? mateTrackSrc.map((q) => [q.lon, q.lat] as Pt) : []), [mateTrackSrc]);
  const mateLon = mate?.pos.lon;
  const mateLat = mate?.pos.lat;
  const mateHdg = mate?.pos.heading ?? 0;
  const mateName = mate?.name ?? "";
  const mateMarker = useMemo(
    () => (mateLon != null && mateLat != null ? { lon: mateLon, lat: mateLat, heading: mateHdg, name: mateName } : null),
    [mateLon, mateLat, mateHdg, mateName],
  );

  // ── Tarjeta del mate: expandir, nombres de aeropuertos, distancia ──
  const [mateExpanded, setMateExpanded] = useState(false);
  const [mateAirports, setMateAirports] = useState<Record<string, AirportBrief>>({});
  useEffect(() => {
    const want = [mate?.origin, mate?.dest].filter((x): x is string => !!x && !mateAirports[x]);
    if (want.length === 0) return;
    void api.lookupAirports(want)
      .then((list) => setMateAirports((prev) => ({ ...prev, ...Object.fromEntries(list.map((a) => [a.icao, a])) })))
      .catch(() => {});
  }, [mate?.origin, mate?.dest, mateAirports]);
  const mateDistance = useMemo(() => {
    if (!own || !mate) return null;
    return Math.round(gcNm([own.lon, own.lat], [mate.pos.lon, mate.pos.lat]));
  }, [own, mate]);

  // ── Mapa: seguir / encuadrar ──
  const [follow, setFollow] = useState(false);
  const [fitSignal, setFitSignal] = useState(0);
  const [focusFriend, setFocusFriend] = useState(0);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const [cardH, setCardH] = useState(260);
  useEffect(() => {
    const el = cardRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setCardH(e.contentRect.height + 24));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const launchFlight = useMemo(
    () => dispatchFlight(ofp?.originIcao, ofp?.destinationIcao, ofp?.aircraftIcao, ofp),
    [ofp],
  );
  const launchPlan = useDispatchPlan(launchFlight);
  const trip = useDispatchStore((s) => s.trip);
  // (03/11) Etapa 8: alternativo y livery en las filas de siempre.
  // Con un viaje activo estas filas siguen hablando del OFP, no del viaje.
  const altnNote = planMatches && ofp?.alternateIcaos?.length ? ` · ALTN ${ofp.alternateIcaos.join(", ")}` : "";
  const liveryLine = planMatches && !trip ? liveryNote(launchPlan) : null;

  // ── Columna «Para este vuelo» ──
  const simIcaos = useSimContentStore((s) => s.icaos);
  const sceneryOf = (icao: string | null) => {
    if (!icao) return undefined;
    const m = sceneriesFor(icao, packages);
    return m.find((p) => p.enabled !== false) ?? m[0];
  };
  const defaultScenery = (icao: string | null) =>
    t(icao && simIcaos.has(icao.toUpperCase()) ? "inflight.need.detailed_scenery" : "inflight.need.default_scenery");
  const depPkg = sceneryOf(originIcao);
  const arrPkg = sceneryOf(destIcao);
  const depPhoto = useAirportPhoto(originIcao, depPkg?.airportName ?? airports[originIcao ?? ""]?.name ?? null);
  const arrPhoto = useAirportPhoto(destIcao, arrPkg?.airportName ?? airports[destIcao ?? ""]?.name ?? null);
  const checks = pre.checks;
  const reg =
    (live ? status?.aircraftRegistration : null) ?? doneEntry?.aircraftRegistration ?? (planMatches ? ofp?.aircraftReg : null) ?? null;
  const acPhoto = useAircraftPhoto(reg ?? null);
  const acTitle =
    (live && liveEntry ? cleanAircraftLabel(liveEntry.aircraftTitle) : "") ||
    (doneEntry ? cleanAircraftLabel(doneEntry.aircraftTitle) : "") ||
    (live ? status?.aircraftIcao : null) ||
    ofp?.aircraftIcao ||
    "—";
  const mx = checks.find((c) => c.id === "mx");
  const ferry = checks.find((c) => c.id === "ferry");
  const gsxRow = (icao: string | null) =>
    icao ? (
      <DotLabel tone={gsxIcaos.has(icao.toUpperCase()) ? "ok" : "off"} label={gsxIcaos.has(icao.toUpperCase()) ? "GSX" : t("inflight.need.no_gsx")} />
    ) : null;
  const legRight = (leg: "dep" | "arr", icao: string | null) => {
    const st = sceneryState(checks, leg);
    return (
      <>
        {st ? <StatusLine tone={st.tone} label={t(st.key)} /> : <StatusLine tone="off" label="—" />}
        {gsxRow(icao)}
      </>
    );
  };

  // ── Estado ──
  const phase = phaseDisplay(status?.phaseLabel ?? null);
  const remainingNm =
    own && destPt ? gcNm([own.lon, own.lat], [destPt.lon, destPt.lat]) : status?.distanceNm ?? null;
  const totalNm = originPt && destPt ? gcNm([originPt.lon, originPt.lat], [destPt.lon, destPt.lat]) : ofp?.distanceNm ?? null;
  const progress = remainingNm != null && totalNm ? Math.max(0, Math.min(1, 1 - remainingNm / totalNm)) : 0;
  const gs = status?.currentGroundSpeedKt ?? 0;
  const etaMs = live && remainingNm != null && gs > 60 ? now + (remainingNm / gs) * 3600_000 : null;
  const resultEntry = landedLive ? liveEntry : doneEntry;
  const startClips = useLandingClipsStore((s) => s.start);
  const clips = useLandingClipsStore((s) => s.clips);
  useEffect(() => startClips(), [startClips]);
  const clip = useMemo(() => (resultEntry ? clipForFlight(clips, resultEntry) : null), [clips, resultEntry]);
  const [playing, setPlaying] = useState(false);
  const warns = checks.filter((c) => c.status === "warn");

  const refreshPlan = async () => {
    try {
      await useSimBriefStore.getState().refresh();
      push({ kind: "success", title: t("inflight.plan.refreshed") });
    } catch (e) {
      push({ kind: "error", title: t("inflight.plan.refresh_failed"), message: String(e) });
    }
  };

  const callsign = (planMatches ? ofp?.callsign ?? ofp?.flightNumber : null) ?? doneEntry?.callsign ?? liveEntry?.callsign ?? null;
  const subtitle = live
    ? landedLive
      ? t("inflight.sub.landed")
      : t("inflight.sub.live")
    : doneEntry
      ? t("inflight.sub.done")
      : ofp
        ? t("inflight.sub.next")
        : t("inflight.sub.empty");

  return (
    <section className="flex h-full flex-col gap-3 overflow-hidden">
      <header className="flex shrink-0 items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight text-sf-text">
            {t("inflight.title")}
            {live && (
              <span className="inline-flex items-center gap-1.5 rounded-md bg-rose-500/15 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide text-rose-300 ring-1 ring-rose-500/30">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-rose-400" />
                {t("inflight.live")}
              </span>
            )}
          </h1>
          <p className="text-sm text-sf-muted">{subtitle}</p>
        </div>
      </header>

      <div className="relative min-h-0 flex-1 overflow-hidden rounded-2xl border border-sf-border bg-[#0e0e10]">
        <InFlightMap
          origin={originPt}
          dest={destPt}
          fixes={fixes}
          own={own}
          ownTrack={ownTrack}
          friend={mateMarker}
          friendTrack={mateTrack}
          follow={follow}
          fitSignal={fitSignal}
          focusFriend={focusFriend}
          bottomPad={cardH}
          onUserMove={() => setFollow(false)}
        />

        {/* El otro piloto, si está volando. */}
        {mate && (
          <div className="absolute left-3 top-3 max-w-[22rem] rounded-xl border border-fuchsia-500/30 bg-sf-surface/90 shadow-lg backdrop-blur">
            <button
              type="button"
              onClick={() => setMateExpanded((v) => !v)}
              onDoubleClick={() => {
                setFollow(false);
                setFocusFriend((n) => n + 1);
              }}
              title={mateExpanded ? undefined : t("inflight.mate.expand")}
              className="flex w-full items-center gap-2.5 px-3 py-2 text-left hover:border-fuchsia-400/60"
            >
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-fuchsia-500/20 text-fuchsia-200 ring-1 ring-fuchsia-500/40">
                <Plane className="h-4 w-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <span className="truncate text-xs font-semibold text-sf-text">
                    {mate.name}
                  </span>
                  {mate.phase && (
                    <span className="shrink-0 rounded bg-fuchsia-500/15 px-1.5 py-0.5 text-[10px] font-medium text-fuchsia-300">
                      {phaseDisplay(mate.phase).short}
                    </span>
                  )}
                </span>
                <span className="mt-0.5 flex items-center gap-1.5 font-mono text-xs text-sf-text">
                  <span className="font-semibold">{mate.origin ?? "····"}</span>
                  <span className="text-fuchsia-400">→</span>
                  <span className="font-semibold">{mate.dest ?? "····"}</span>
                  {mate.aircraft && (
                    <span className="text-sf-muted">· {mate.aircraft}</span>
                  )}
                </span>
              </span>
              <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 text-sf-muted transition-transform", mateExpanded && "rotate-180")} />
            </button>
            {mateExpanded && (
              <div className="border-t border-fuchsia-500/20 px-3 pb-2.5 pt-2">
                {(mate.origin || mate.dest) && (
                  <div className="mb-2 flex items-start gap-2">
                    <MapPin className="mt-0.5 h-3.5 w-3.5 shrink-0 text-fuchsia-400" />
                    <div className="min-w-0 text-[11px]">
                      {mate.origin && (
                        <div className="truncate text-sf-text">
                          {mateAirports[mate.origin]?.municipality || mateAirports[mate.origin]?.name || mate.origin}
                        </div>
                      )}
                      {mate.dest && (
                        <div className="truncate text-sf-muted">
                          → {mateAirports[mate.dest]?.municipality || mateAirports[mate.dest]?.name || mate.dest}
                        </div>
                      )}
                    </div>
                  </div>
                )}
                <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px]">
                  <span className="text-sf-muted">{t("inflight.mate.alt")} <span className="font-mono text-sf-text">{u.fmt.altitude(mate.pos.alt)}</span></span>
                  <span className="text-sf-muted">{t("inflight.mate.gs")} <span className="font-mono text-sf-text">{u.fmt.speed(mate.pos.gs)}</span></span>
                  {mateDistance != null && (
                    <span className="text-sf-muted">{t("inflight.mate.dist")} <span className="font-mono text-sf-text">{mateDistance} NM</span></span>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => {
                    setFollow(false);
                    setFocusFriend((n) => n + 1);
                  }}
                  className="mt-2 w-full rounded-lg bg-fuchsia-500/15 px-2.5 py-1.5 text-center text-[11px] font-medium text-fuchsia-300 hover:bg-fuchsia-500/25"
                >
                  {t("inflight.mate.center")}
                </button>
              </div>
            )}
          </div>
        )}

        {/* Controles del mapa. */}
        <div className="absolute right-3 top-12 flex flex-col gap-1.5">
          {own && (
            <button
              type="button"
              onClick={() => setFollow((v) => !v)}
              title={t("inflight.map.follow")}
              className={cn(
                "flex h-8 w-8 items-center justify-center rounded-lg border bg-sf-surface/90 shadow backdrop-blur",
                follow ? "border-amber-400/60 text-amber-300" : "border-sf-border text-sf-muted hover:text-sf-text",
              )}
            >
              <Crosshair className="h-4 w-4" />
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              setFollow(false);
              setFitSignal((n) => n + 1);
            }}
            title={t("inflight.map.fit")}
            className="flex h-8 w-8 items-center justify-center rounded-lg border border-sf-border bg-sf-surface/90 text-sf-muted shadow backdrop-blur hover:text-sf-text"
          >
            <Maximize2 className="h-4 w-4" />
          </button>
        </div>

        {/* Tarjeta de abajo (como la de Bob). */}
        <div
          ref={cardRef}
          className="absolute inset-x-3 bottom-3 max-h-[50%] overflow-y-auto rounded-2xl border border-sf-border bg-sf-surface/95 shadow-2xl shadow-black/40 backdrop-blur"
        >
          {!originIcao && !destIcao ? (
            <div className="flex flex-wrap items-center justify-between gap-3 p-4">
              <div>
                <div className="text-sm font-semibold text-sf-text">{t("inflight.empty.title")}</div>
                <div className="mt-1 text-xs text-sf-muted">{t("inflight.empty.body")}</div>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void refreshPlan()}
                  className="inline-flex items-center gap-2 rounded-xl border border-sf-border px-4 py-2.5 text-sm font-medium text-sf-text hover:border-brand-500/40"
                >
                  <RefreshCw className="h-4 w-4" />
                  {t("inflight.plan.refresh")}
                </button>
                {/* (02/11) Etapa 6: sin OFP, la secuencia por defecto. */}
                {trip && <DispatchSelector flight={null} className="w-60" />}
                <LaunchButton aircraftIcao={null} />
              </div>
            </div>
          ) : (
            <div className="grid grid-cols-1 divide-y divide-sf-border lg:grid-cols-[1.05fr_1.4fr_0.95fr] lg:divide-x lg:divide-y-0">
              {/* Vuelo */}
              <Column title={t("inflight.col.flight")}>
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-sm font-semibold text-sf-text">{callsign ?? "—"}</span>
                  <span className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      onClick={() => void refreshPlan()}
                      title={t("inflight.plan.refresh")}
                      className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                    >
                      <RefreshCw className="h-3.5 w-3.5" />
                    </button>
                    {planMatches && ofp && !live && (
                      <button
                        type="button"
                        onClick={() => void useSimBriefStore.getState().excludeOfp(ofp.ofpId)}
                        title={t("inflight.plan.not_mine")}
                        className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-rose-300"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </span>
                </div>
                <div className="mt-2 flex items-center gap-2">
                  <div className="min-w-0">
                    <div className="font-mono text-xl font-semibold leading-none text-sf-text">{originIcao ?? "····"}</div>
                    <div className="mt-0.5 truncate text-[10px] text-sf-muted">{city(originIcao, ofp?.originName)}</div>
                  </div>
                  <div className="relative mx-0.5 h-px min-w-[2rem] flex-1 bg-sf-border">
                    <span className="absolute -top-[2.5px] left-0 h-[6px] w-[6px] rounded-full bg-sf-muted" />
                    <span className="absolute -top-[2.5px] right-0 h-[6px] w-[6px] rounded-full bg-sf-muted" />
                    {(live || resultEntry) && (
                      <span
                        className="absolute -top-[7px] -ml-[7px] text-amber-300"
                        style={{ left: `${(resultEntry ? 1 : progress) * 100}%` }}
                      >
                        <Plane className="h-[14px] w-[14px] rotate-45" />
                      </span>
                    )}
                  </div>
                  <div className="min-w-0 text-right">
                    <div className="font-mono text-xl font-semibold leading-none text-sf-text">{destIcao ?? "····"}</div>
                    <div className="mt-0.5 truncate text-[10px] text-sf-muted">{city(destIcao, ofp?.destinationName)}</div>
                  </div>
                </div>
                <div className="mt-2.5 grid grid-cols-2 gap-x-3 gap-y-2">
                  <Fact label={t("inflight.fact.aircraft")} value={acTitle} />
                  <Fact
                    label={t("inflight.fact.registration")}
                    value={
                      reg ? (
                        <button
                          type="button"
                          onClick={() => useAppStore.getState().requestHangarFocus(reg)}
                          className="font-mono text-brand-300 hover:text-brand-200 hover:underline"
                        >
                          {reg}
                        </button>
                      ) : (
                        "—"
                      )
                    }
                  />
                  <Fact label={t("inflight.fact.distance")} value={u.fmt.distance(ofp?.distanceNm ?? totalNm)} />
                  <Fact label={t("inflight.fact.ete")} value={fmtEte(planMatches ? ofp?.estTimeEnrouteS : doneEntry?.flightTimeS)} />
                </div>
                {planMatches && ofp?.route && (
                  <div className="mt-2">
                    <div className="text-[10px] text-sf-muted">{t("inflight.fact.route")}</div>
                    <div className="mt-0.5 line-clamp-2 font-mono text-[10px] leading-relaxed text-sf-text/90">{ofp.route}</div>
                  </div>
                )}
              </Column>

              {/* Para este vuelo */}
              <Column title={t("inflight.col.needs")}>
                <div className="space-y-1">
                  <NeedRow
                    thumb={depPhoto}
                    fallback={<PlaneLanding className="h-4 w-4 -scale-x-100 text-sf-muted" />}
                    title={depPkg?.airportName ?? airports[originIcao ?? ""]?.name ?? originIcao ?? "—"}
                    sub={`${t("inflight.need.departure")} · ${depPkg?.creator ?? (depPkg ? depPkg.title : defaultScenery(originIcao))}`}
                    right={legRight("dep", originIcao)}
                    onClick={() => {
                      if (depPkg) {
                        useCommunityStore.getState().setFocused(depPkg.folderName);
                        useAppStore.getState().setView("map");
                      } else {
                        openPreflight(true);
                      }
                    }}
                  />
                  <NeedRow
                    thumb={arrPhoto}
                    fallback={<PlaneLanding className="h-4 w-4 text-sf-muted" />}
                    title={arrPkg?.airportName ?? airports[destIcao ?? ""]?.name ?? destIcao ?? "—"}
                    sub={`${t("inflight.need.arrival")} · ${arrPkg?.creator ?? (arrPkg ? arrPkg.title : defaultScenery(destIcao))}${altnNote}`}
                    right={legRight("arr", destIcao)}
                    onClick={() => {
                      if (arrPkg) {
                        useCommunityStore.getState().setFocused(arrPkg.folderName);
                        useAppStore.getState().setView("map");
                      } else {
                        openPreflight(true);
                      }
                    }}
                  />
                  <NeedRow
                    thumb={acPhoto}
                    fallback={<Plane className="h-4 w-4 text-sf-muted" />}
                    title={acTitle}
                    sub={`${t("inflight.need.aircraft")}${reg ? ` · ${reg}` : ""}${liveryLine ? ` · ${liveryLine}` : ""}`}
                    right={
                      <>
                        {mx ? (
                          <StatusLine tone={mx.status === "ok" ? "ok" : mx.status === "warn" ? "bad" : "off"} label={t(mx.status === "ok" ? "inflight.need.mx_ok" : mx.status === "warn" ? "inflight.need.mx_nogo" : "inflight.need.mx_info")} />
                        ) : (
                          <StatusLine tone="off" label="—" />
                        )}
                        {ferry && (
                          <DotLabel tone={ferry.status === "ok" ? "ok" : ferry.status === "warn" ? "warn" : "off"} label={t(ferry.status === "ok" ? "inflight.need.here" : "inflight.need.ferry")} />
                        )}
                      </>
                    }
                    onClick={() => (reg ? useAppStore.getState().requestHangarFocus(reg) : openPreflight(true))}
                  />
                </div>
              </Column>

              {/* Estado */}
              <Column title={t("inflight.col.state")} className="flex flex-col">
                {resultEntry ? (
                  <div className="flex flex-1 flex-col">
                    <div className="text-[11px] text-sf-muted">{t("inflight.state.landing")}</div>
                    <div className="mt-1 flex items-baseline gap-2">
                      <span className={cn("font-mono text-3xl font-semibold", fpmTone(-Math.abs(resultEntry.landingFpm ?? 0)))}>
                        {u.fmt.vs(-Math.abs(resultEntry.landingFpm ?? 0))}
                      </span>
                      {resultEntry.scoreGrade && (
                        <span className="rounded-md bg-sf-raised px-2 py-0.5 text-xs font-bold text-sf-text ring-1 ring-sf-border">
                          {resultEntry.scoreGrade}
                        </span>
                      )}
                    </div>
                    {resultEntry.bounced && <div className="mt-1 text-[11px] text-amber-300">↑ {t("fb.block.bounce_detected")}</div>}
                    <div className="mt-auto flex flex-wrap gap-2 pt-3">
                      {clip && (
                        <button
                          type="button"
                          onClick={() => setPlaying(true)}
                          className="inline-flex items-center gap-1.5 rounded-xl bg-brand-500 px-3 py-2 text-xs font-semibold text-slate-950 hover:bg-brand-400"
                        >
                          <PlayCircle className="h-4 w-4" />
                          {t("fb.clip.button")}
                        </button>
                      )}
                      {/* El FlightBook lo lista al cerrarse (motores apagados). */}
                      {resultEntry.endedAt != null && (
                        <button
                          type="button"
                          onClick={() => useAppStore.getState().requestFlightFocus(resultEntry.id)}
                          className="inline-flex items-center gap-1.5 rounded-xl border border-sf-border px-3 py-2 text-xs font-medium text-sf-text hover:border-brand-500/40"
                        >
                          <BookOpen className="h-4 w-4" />
                          {t("inflight.state.open_flightbook")}
                        </button>
                      )}
                    </div>
                  </div>
                ) : live ? (
                  <div className="flex flex-1 flex-col">
                    <span className={cn("inline-flex w-fit items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-semibold", phase.toneClass)}>
                      <span className={cn("h-1.5 w-1.5 rounded-full", phase.dotClass)} />
                      {phase.short}
                    </span>
                    <div className="mt-2 grid grid-cols-2 gap-x-3 gap-y-2">
                      <Fact label={t("inflight.fact.altitude")} value={u.fmt.altitude(status?.currentAltFt ?? null)} />
                      <Fact label={t("inflight.fact.speed")} value={u.fmt.speed(status?.currentGroundSpeedKt ?? null)} />
                      <Fact label={t("inflight.fact.remaining")} value={u.fmt.distance(remainingNm)} />
                      <Fact label={t("inflight.fact.eta")} value={etaMs ? fmtClock(etaMs) : "—"} />
                    </div>
                    <div className="mt-auto pt-3">
                      <div className="h-1 overflow-hidden rounded-full bg-sf-raised">
                        <div className="h-full rounded-full bg-amber-400" style={{ width: `${Math.round(progress * 100)}%` }} />
                      </div>
                      <div className="mt-0.5 text-right text-[10px] tabular-nums text-sf-muted">{Math.round(progress * 100)}%</div>
                    </div>
                  </div>
                ) : (
                  <div className="flex flex-1 flex-col">
                    {pre.ready ? (
                      <div className="flex items-center gap-2 text-sm font-semibold text-emerald-300">
                        <CheckCircle2 className="h-4 w-4" />
                        {t("inflight.state.ready")}
                      </div>
                    ) : (
                      <div className="flex items-center gap-2 text-sm font-semibold text-amber-300">
                        <TriangleAlert className="h-4 w-4" />
                        {t("inflight.state.issues", { n: String(pre.issues) })}
                      </div>
                    )}
                    {warns.length > 0 && (
                      <ul className="mt-2 space-y-1">
                        {warns.slice(0, 3).map((c) => (
                          <li key={c.id} className="truncate text-xs text-sf-muted">
                            · {t(c.titleKey, c.titleArgs)}
                          </li>
                        ))}
                      </ul>
                    )}
                    {/* (02/11) Etapa 6: lo principal antes de volar es lanzar. */}
                    <DispatchSelector flight={launchFlight} className="mt-auto pt-3" />
                    <LaunchButton aircraftIcao={ofp?.aircraftIcao} flight={launchFlight} className="mt-2 w-full" />
                    <button
                      type="button"
                      onClick={() => openPreflight(true)}
                      className="mt-2 inline-flex w-full items-center justify-center gap-2 rounded-xl border border-sf-border px-4 py-2 text-sm font-medium text-sf-text hover:border-brand-500/40"
                    >
                      <ClipboardCheck className="h-4 w-4" />
                      {t("inflight.state.open_preflight")}
                    </button>
                  </div>
                )}
              </Column>
            </div>
          )}
        </div>
      </div>
      {playing && clip && resultEntry && (
        <ClipPlayerModal
          clip={clip}
          title={`${resultEntry.originIcao ?? "?"} → ${resultEntry.destinationIcao ?? "?"}`}
          onClose={() => setPlaying(false)}
        />
      )}
    </section>
  );
}
