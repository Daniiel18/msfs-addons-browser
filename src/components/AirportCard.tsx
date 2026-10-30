import { useEffect, useMemo, useState, type ReactNode } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  ArrowDown,
  ArrowUp,
  Crosshair,
  Lightbulb,
  Loader2,
  Plane,
  PlaneLanding,
  PlaneTakeoff,
  Play,
  Search,
  Users,
  X,
} from "lucide-react";
import { api, isTauri } from "../lib/tauri";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { GSX_KIND_COLORS } from "../lib/gsxKinds";
import { gsxSearchUrl, openInFlightsimTab } from "../lib/flightsimTab";
import { GsxParkingList, gsxSections, type GsxListAirline } from "./MapAirportCard";
import { AirlineLogo } from "./AirlineLogo";
import { gateKey } from "../lib/gsxAirlines";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import type { AirportCardData, AirportRunway, GsxAirportParkings, LandingClip } from "../lib/types";

/**
 * (26/10) Filtro de aerolínea del FlightBook: la tarjeta cuenta sólo esos
 * vuelos (y el clip, sólo de los aviones que volaron con ella).
 */
export interface AirportCardAirline {
  /** `canonicalAirlineKey`, la misma clave que la lista de vuelos. */
  key: string;
  icao: string | null;
  name: string;
  flightIds: number[];
  registrations: string[];
}

/**
 * (26/10) Lo que la tarjeta pide enfocar en el mapa: el aeropuerto entero,
 * una pista (línea entre cabeceras) o un punto (gate, parking, helipuerto).
 * Coordenadas `[lat, lon]`.
 */
export type AirportFocus =
  | { kind: "airport"; icao: string; id: string; label: string; at: [number, number] }
  | {
      kind: "runway";
      icao: string;
      id: string;
      label: string;
      ends: [string, string];
      from: [number, number];
      to: [number, number];
    }
  | { kind: "spot"; icao: string; id: string; label: string; at: [number, number]; color: string };

type Tab = "summary" | "runways" | "gates";

/**
 * (24/10) Tarjeta de aeropuerto dentro del mapa del FlightBook (clic en un
 * aeropuerto). (26/10) Mide lo que su contenido, con pestañas Resumen · Pistas
 * · Gates; las pistas, helipuertos, gates y parkings se enfocan en el mapa
 * satélite. La foto sale de la caché que el mapa precarga y, si Wikipedia no
 * tiene una foto real, se ve el aeropuerto desde el satélite.
 */
export function AirportCard({
  icao,
  name: hintName,
  airline = null,
  focusedId,
  onFocus,
  onClose,
}: {
  icao: string;
  name?: string | null;
  /** (26/10) Aerolínea filtrada en el FlightBook (o `null`: todas). */
  airline?: AirportCardAirline | null;
  /** `id` del foco activo en el mapa (para marcarlo en la lista). */
  focusedId?: string | null;
  onFocus: (f: AirportFocus | null) => void;
  onClose: () => void;
}) {
  const [data, setData] = useState<AirportCardData | null>(null);
  const [error, setError] = useState(false);
  const [photo, setPhoto] = useState<PhotoState>(() => photoStateOf(icao));
  const [clip, setClip] = useState<LandingClip | null>(null);
  const [tab, setTab] = useState<Tab>("summary");
  const [runways, setRunways] = useState<AirportRunway[] | null>(null);
  const [gsx, setGsx] = useState<GsxAirportParkings | null | undefined>(undefined);

  // Lo que no depende de la aerolínea: foto, pistas y gates del aeropuerto.
  useEffect(() => {
    let alive = true;
    setPhoto(photoStateOf(icao));
    setTab("summary");
    setRunways(null);
    setGsx(undefined);
    void airportPhotoUrl(icao, hintName ?? null).then((url) => alive && setPhoto(url));
    void api
      .airportRunways(icao)
      .then((r) => alive && setRunways(r))
      .catch(() => alive && setRunways([]));
    void api
      .gsxAirportParkings(icao)
      .then((g) => alive && setGsx(g && g.parkings.length > 0 ? g : null))
      .catch(() => alive && setGsx(null));
    return () => {
      alive = false;
    };
  }, [icao, hintName]);

  // Los números (de todas las aerolíneas o de la filtrada). Al cambiar el
  // filtro se recalcula sin vaciar la tarjeta.
  const airlineKey = airline ? airline.flightIds.join(",") : "";
  useEffect(() => {
    let alive = true;
    setError(false);
    const regs = airline ? new Set(airline.registrations.map(normReg)) : null;
    api
      .airportCard(icao, airline?.flightIds ?? null)
      .then((d) => {
        if (!alive) return;
        setData(d);
        void api
          .listLandingClips()
          .then((clips) => alive && setClip(pickClip(clips, d, regs)))
          .catch(() => {});
      })
      .catch(() => alive && setError(true));
    return () => {
      alive = false;
    };
    // `airlineKey` resume `airline`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [icao, airlineKey]);

  const title = data?.name ?? hintName ?? icao;
  const place = [data?.municipality, countryName(data?.isoCountry)].filter(Boolean).join(", ");
  const flights = (data?.departures ?? 0) + (data?.arrivals ?? 0);
  const strips = runways?.filter((r) => !r.helipad) ?? [];
  const helipads = runways?.filter((r) => r.helipad) ?? [];
  const at: [number, number] | null =
    data?.latitude != null && data?.longitude != null ? [data.latitude, data.longitude] : null;

  const focusSpot = (name: string | null) => {
    const p = name ? gsx?.parkings.find((x) => x.name === name) : undefined;
    onFocus(
      p
        ? {
            kind: "spot",
            icao,
            id: `gsx:${p.name}`,
            label: p.name,
            at: [p.lat, p.lon],
            color: GSX_KIND_COLORS[p.kind],
          }
        : null,
    );
  };
  // (26/10) Pestaña Gates con filtro de aerolínea: sus gates en el perfil y
  // los que usaste con ella aquí (salida o llegada de tus vuelos).
  const entries = useFlightLogStore((s) => s.entries);
  const gsxAirline = useMemo<GsxListAirline | null>(() => {
    if (!airline) return null;
    const ids = new Set(airline.flightIds);
    const here = icao.toUpperCase();
    const used = new Map<string, { name: string; count: number }>();
    const add = (gate: string | null | undefined) => {
      const name = gate?.trim();
      if (!name) return;
      const prev = used.get(gateKey(name));
      if (prev) prev.count++;
      else used.set(gateKey(name), { name, count: 1 });
    };
    for (const e of entries) {
      if (!ids.has(e.id)) continue;
      if (e.originIcao?.toUpperCase() === here) add(e.departureGate);
      if (e.destinationIcao?.toUpperCase() === here) add(e.arrivalGate);
    }
    return {
      key: airline.key,
      icao: airline.icao,
      name: airline.name,
      used: [...used.values()].sort((a, b) => b.count - a.count),
    };
    // `airlineKey` resume los vuelos de `airline`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [airline?.key, airline?.name, airlineKey, entries, icao]);
  const gatesCount = gsx
    ? gsxAirline
      ? gsxSections(gsx, gsxAirline, null).visible.length
      : gsx.parkings.length
    : gsx;

  // El gate más usado, si el perfil de GSX lo tiene (para enfocarlo).
  const topGateSpot = useMemo(() => {
    const g = data?.topGate?.trim().toLowerCase();
    return g ? gsx?.parkings.find((p) => p.name.trim().toLowerCase() === g) ?? null : null;
  }, [data?.topGate, gsx]);

  return (
    <div
      className="pointer-events-auto absolute right-3 top-12 z-20 flex max-h-[calc(100%-3.75rem)] w-[320px] max-w-[calc(100%-1.5rem)] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface/95 text-[13px] text-sf-text shadow-2xl backdrop-blur"
      onMouseDown={(e) => e.stopPropagation()}
    >
      {/* Foto (o el aeropuerto visto desde el satélite) */}
      <div className="relative h-32 shrink-0 overflow-hidden bg-gradient-to-br from-sky-900/60 via-slate-900 to-slate-950">
        {photo ? (
          <FadeImg src={photo} />
        ) : photo === null && at ? (
          <>
            <SatelliteThumb lat={at[0]} lon={at[1]} />
            <span className="absolute left-2 top-2 rounded-md bg-slate-950/70 px-1.5 py-0.5 text-[10px] text-sf-text/90">
              {t("fb.airport.satellite")}
            </span>
          </>
        ) : photo === undefined ? (
          <div className="absolute inset-0 flex items-center justify-center text-sf-muted/60">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : null}
        <div className="absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-slate-950/80 to-transparent" />
        <span className="absolute bottom-2 left-3 font-mono text-xs font-semibold text-emerald-300">{icao}</span>
        <div className="absolute right-2 top-2 flex gap-1">
          {at && (
            <button
              onClick={() => onFocus({ kind: "airport", icao, id: `apt:${icao}`, label: icao, at })}
              title={t("fb.airport.focus_airport")}
              className={cn(
                "rounded-md p-1 text-sf-text hover:bg-slate-950",
                focusedId === `apt:${icao}` ? "bg-brand-500 text-slate-950 hover:bg-brand-400" : "bg-slate-950/70",
              )}
            >
              <Crosshair className="h-4 w-4" />
            </button>
          )}
          <button
            onClick={onClose}
            title={t("fb.airport.close")}
            className="rounded-md bg-slate-950/70 p-1 text-sf-text hover:bg-slate-950"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      <div className="shrink-0 px-3 pt-2.5">
        <div className="text-[15px] font-medium leading-tight">{title}</div>
        <div className="text-xs text-sf-muted">
          <span className="font-mono">{icao}</span>
          {place && <> · {place}</>}
        </div>
        {airline && (
          <span
            title={t("fb.airport.only_airline_hint", { name: airline.name })}
            className="mt-1.5 inline-flex max-w-full items-center gap-1.5 rounded-full border border-brand-500/40 bg-brand-500/10 px-2 py-0.5 text-[11px] font-medium text-brand-100"
          >
            <AirlineLogo icao={airline.icao} name={airline.name} size={14} />
            <span className="truncate">{t("fb.airport.only_airline", { name: airline.name })}</span>
          </span>
        )}
        <div role="tablist" className="mt-2.5 flex gap-1 rounded-lg bg-sf-raised/60 p-0.5">
          <TabButton active={tab === "summary"} onClick={() => setTab("summary")}>
            {t("fb.airport.tab.summary")}
          </TabButton>
          <TabButton active={tab === "runways"} onClick={() => setTab("runways")} count={runways?.length}>
            {t("fb.airport.tab.runways")}
          </TabButton>
          <TabButton active={tab === "gates"} onClick={() => setTab("gates")} count={gatesCount}>
            {t("fb.airport.tab.gates")}
          </TabButton>
        </div>
      </div>

      <div className="min-h-0 overflow-y-auto pb-3 pt-2.5">
        {tab === "summary" && (
          <div className="px-3">
            {error && <p className="text-xs text-rose-300">{t("fb.airport.error")}</p>}
            {!data && !error && <Spinner />}
            {data && (
              <>
                {data.parked.length > 0 && (
                  <div className="mb-2 flex flex-wrap gap-1.5">
                    {data.parked.map((reg) => (
                      <span
                        key={reg}
                        className="inline-flex items-center gap-1 rounded-md bg-sf-raised px-2 py-0.5 text-[11px] text-sf-text"
                      >
                        <Plane className="h-3 w-3 text-emerald-300" />
                        {t("fb.airport.parked", { reg })}
                      </span>
                    ))}
                  </div>
                )}

                {flights === 0 ? (
                  <p className="text-xs text-sf-muted">{t("fb.airport.no_flights")}</p>
                ) : (
                  <>
                    <div className="grid grid-cols-2 gap-1.5">
                      <Tile title={t("fb.airport.departures")}>
                        <PlaneTakeoff className="h-4 w-4 text-sky-300" />
                        <span className="text-lg font-medium">{fmtInt(data.departures)}</span>
                      </Tile>
                      <Tile title={t("fb.airport.arrivals")}>
                        <PlaneLanding className="h-4 w-4 text-emerald-300" />
                        <span className="text-lg font-medium">{fmtInt(data.arrivals)}</span>
                      </Tile>
                      <Tile title={t("fb.airport.pax")}>
                        <div>
                          <div className="flex items-center gap-1.5">
                            <Users className="h-4 w-4 text-sf-muted" />
                            <span className="text-lg font-medium">
                              {fmtInt(data.paxBoarded + data.paxDisembarked)}
                            </span>
                          </div>
                          <div className="flex gap-2 text-[11px] text-sf-muted">
                            <span className="inline-flex items-center" title={t("fb.airport.pax_up")}>
                              <ArrowUp className="h-3 w-3" />
                              {fmtInt(data.paxBoarded)}
                            </span>
                            <span className="inline-flex items-center" title={t("fb.airport.pax_down")}>
                              <ArrowDown className="h-3 w-3" />
                              {fmtInt(data.paxDisembarked)}
                            </span>
                          </div>
                        </div>
                      </Tile>
                      <Tile title={t("fb.airport.best_landing")}>
                        <div className="min-w-0">
                          <div className="text-[11px] text-sf-muted">{t("fb.airport.best_landing")}</div>
                          {data.bestFpm != null ? (
                            <>
                              <div className={`font-mono text-lg font-medium ${fpmColor(data.bestFpm)}`}>
                                {fmtFpm(data.bestFpm)} fpm
                              </div>
                              <div className="flex flex-wrap gap-x-1 text-[11px] text-sf-muted">
                                {data.avgFpm != null && (
                                  <span>{t("fb.airport.avg", { fpm: fmtFpm(data.avgFpm) })}</span>
                                )}
                                {clip && (
                                  <button
                                    onClick={() => void api.openLocalPath(clip.path).catch(() => {})}
                                    className="inline-flex items-center gap-0.5 text-brand-300 hover:underline"
                                  >
                                    · <Play className="h-3 w-3" />
                                    {t("fb.airport.see_clip")}
                                  </button>
                                )}
                              </div>
                            </>
                          ) : (
                            <div className="text-[11px] text-sf-muted">{t("fb.airport.no_landings")}</div>
                          )}
                        </div>
                      </Tile>
                    </div>

                    <div className="mt-3 text-xs text-sf-muted">{t("fb.airport.money_title", { icao })}</div>
                    {data.money.length === 0 ? (
                      <p className="mt-1 text-xs text-sf-muted">{t("fb.airport.money_none")}</p>
                    ) : (
                      <div className="mt-1 grid grid-cols-[1fr_auto] gap-y-0.5 text-xs">
                        {data.money.map((m) => (
                          <MoneyLine key={m.group} label={t(`fb.airport.money.${m.group}`)} usd={m.usd} />
                        ))}
                        <span className="mt-0.5 border-t border-sf-border pt-0.5 font-medium">
                          {t("fb.airport.total")}
                        </span>
                        <span className="mt-0.5 border-t border-sf-border pt-0.5 text-right font-mono font-medium">
                          {fmtUsd(data.moneyTotal)}
                        </span>
                      </div>
                    )}

                    <div className="mt-3 grid grid-cols-[1fr_auto] gap-y-1 text-xs">
                      {data.topGate && (
                        <>
                          <span className="text-sf-muted">{t("fb.airport.top_gate")}</span>
                          <span className="text-right">
                            {topGateSpot ? (
                              <button
                                onClick={() => focusSpot(topGateSpot.name)}
                                title={t("fb.airport.focus_gate")}
                                className="inline-flex items-center gap-1 text-brand-300 hover:underline"
                              >
                                <Crosshair className="h-3 w-3" />
                                {data.topGate}
                              </button>
                            ) : (
                              data.topGate
                            )}{" "}
                            <span className="text-sf-muted">
                              (
                              {data.topGateCount === 1
                                ? t("fb.airport.times_one")
                                : t("fb.airport.times", { n: String(data.topGateCount) })}
                              )
                            </span>
                          </span>
                        </>
                      )}
                      {data.topRoute && (
                        <>
                          <span className="text-sf-muted">{t("fb.airport.top_route")}</span>
                          <span className="text-right font-mono">
                            {data.topRoute} ×{data.topRouteCount}
                          </span>
                        </>
                      )}
                      {data.lastVisitAt && (
                        <>
                          <span className="text-sf-muted">{t("fb.airport.last_visit")}</span>
                          <span className="text-right">
                            {fmtDay(data.lastVisitAt)}
                            {data.lastVisitLabel && <span className="font-mono"> · {data.lastVisitLabel}</span>}
                          </span>
                        </>
                      )}
                    </div>
                  </>
                )}
              </>
            )}
          </div>
        )}

        {tab === "runways" && (
          <div className="px-3">
            {runways === null ? (
              <Spinner />
            ) : runways.length === 0 ? (
              <p className="text-xs text-sf-muted">{t("fb.airport.runways_none", { icao })}</p>
            ) : (
              <>
                <p className="mb-1.5 text-[11px] text-sf-muted">{t("fb.airport.runways_hint")}</p>
                <div className="space-y-1">
                  {strips.map((r) => (
                    <RunwayRow key={r.ident} icao={icao} r={r} focusedId={focusedId} onFocus={onFocus} />
                  ))}
                </div>
                {helipads.length > 0 && (
                  <>
                    <div className="mb-1 mt-3 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">
                      {t("fb.airport.helipads")} · {helipads.length}
                    </div>
                    <div className="space-y-1">
                      {helipads.map((r) => (
                        <RunwayRow key={r.ident} icao={icao} r={r} focusedId={focusedId} onFocus={onFocus} />
                      ))}
                    </div>
                  </>
                )}
              </>
            )}
          </div>
        )}

        {tab === "gates" &&
          (gsx === undefined ? (
            <Spinner />
          ) : gsx === null ? (
            <div className="px-3 text-xs text-sf-muted">
              <p>{t("fb.airport.gates_none", { icao })}</p>
              <button
                onClick={() => openInFlightsimTab(gsxSearchUrl(icao))}
                className="mt-2 inline-flex items-center gap-1.5 rounded-lg border border-sf-border bg-sf-raised px-2.5 py-1.5 text-[11px] font-medium text-sf-text hover:border-brand-500/40"
              >
                <Search className="h-3 w-3" />
                {t("fb.airport.gsx_search")}
              </button>
            </div>
          ) : (
            <div className="-mx-1">
              <GsxParkingList
                data={gsx}
                picked={focusedId?.startsWith("gsx:") ? focusedId.slice(4) : null}
                onPick={focusSpot}
                airline={gsxAirline}
              />
            </div>
          ))}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  count,
  children,
}: {
  active: boolean;
  onClick: () => void;
  count?: number | null;
  children: ReactNode;
}) {
  return (
    <button
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "flex-1 rounded-md px-2 py-1 text-xs font-medium transition-colors",
        active ? "bg-brand-400 text-slate-950 shadow" : "text-sf-muted hover:text-sf-text",
      )}
    >
      {children}
      {count != null && <span className={active ? "text-slate-950/70" : "text-sf-muted/80"}> · {count}</span>}
    </button>
  );
}

/** Una pista (o helipuerto): su orientación dibujada, medidas y superficie. */
function RunwayRow({
  icao,
  r,
  focusedId,
  onFocus,
}: {
  icao: string;
  r: AirportRunway;
  focusedId?: string | null;
  onFocus: (f: AirportFocus | null) => void;
}) {
  const id = `rwy:${r.ident}`;
  const focus: AirportFocus | null = r.helipad
    ? r.le
      ? { kind: "spot", icao, id, label: r.ident, at: r.le, color: GSX_KIND_COLORS.pad }
      : null
    : r.le && r.he
      ? {
          kind: "runway",
          icao,
          id,
          label: r.ident,
          ends: (r.ident.split("/").concat(["", ""]).slice(0, 2) as [string, string]),
          from: r.le,
          to: r.he,
        }
      : null;
  const heading = r.le && r.he ? bearing(r.le, r.he) : null;
  const active = focusedId === id;
  const details = [
    r.lengthM != null
      ? r.helipad
        ? `${fmtInt(r.lengthM)} m`
        : `${fmtInt(r.lengthM)} m · ${fmtInt(r.lengthFt ?? 0)} ft`
      : null,
    r.widthM != null && !r.helipad ? `× ${fmtInt(r.widthM)} m` : null,
    surfaceLabel(r.surface),
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      disabled={!focus}
      onClick={() => focus && onFocus(active ? null : focus)}
      title={focus ? t("fb.airport.focus_runway") : t("fb.airport.no_coords")}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-lg border px-2.5 py-1.5 text-left transition-colors disabled:cursor-default disabled:opacity-60",
        active
          ? "border-brand-500/50 bg-brand-500/10"
          : "border-transparent bg-sf-raised/70 enabled:hover:border-brand-500/30",
      )}
    >
      <span className="relative flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-slate-950/60">
        {r.helipad ? (
          <span className="text-[11px] font-bold" style={{ color: GSX_KIND_COLORS.pad }}>
            H
          </span>
        ) : (
          <span
            className="h-5 w-1.5 rounded-sm bg-slate-300"
            style={{ transform: `rotate(${heading ?? 0}deg)`, opacity: heading == null ? 0.4 : 1 }}
          />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="font-mono text-[13px] font-semibold">{r.ident}</span>
          {r.lighted && <Lightbulb className="h-3 w-3 text-amber-300" aria-label={t("fb.airport.lighted")} />}
        </span>
        {details && <span className="block truncate text-[11px] text-sf-muted">{details}</span>}
      </span>
      {focus && <Crosshair className={cn("h-3.5 w-3.5 shrink-0", active ? "text-brand-300" : "text-sf-muted")} />}
    </button>
  );
}

function Spinner() {
  return (
    <div className="flex justify-center py-6 text-sf-muted">
      <Loader2 className="h-5 w-5 animate-spin" />
    </div>
  );
}

function FadeImg({ src }: { src: string }) {
  const [loaded, setLoaded] = useState(false);
  return (
    <img
      src={src}
      alt=""
      onLoad={() => setLoaded(true)}
      className={cn(
        "absolute inset-0 h-full w-full object-cover transition-opacity duration-300",
        loaded ? "opacity-100" : "opacity-0",
      )}
    />
  );
}

/**
 * El aeropuerto visto desde el satélite (teselas de Esri World Imagery, las
 * mismas del mapa), centrado en sus coordenadas.
 */
function SatelliteThumb({ lat, lon, zoom = 14 }: { lat: number; lon: number; zoom?: number }) {
  const W = 320;
  const H = 128;
  const n = 2 ** zoom * 256;
  const x = ((lon + 180) / 360) * n;
  const rad = (lat * Math.PI) / 180;
  const y = ((1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2) * n;
  const x0 = x - W / 2;
  const y0 = y - H / 2;
  const tiles: { tx: number; ty: number; left: number; top: number }[] = [];
  for (let tx = Math.floor(x0 / 256); tx <= Math.floor((x0 + W) / 256); tx++) {
    for (let ty = Math.floor(y0 / 256); ty <= Math.floor((y0 + H) / 256); ty++) {
      tiles.push({ tx, ty, left: tx * 256 - x0, top: ty * 256 - y0 });
    }
  }
  const max = 2 ** zoom;
  return (
    <div className="absolute inset-0">
      {tiles.map(({ tx, ty, left, top }) => (
        <img
          key={`${tx}/${ty}`}
          alt=""
          draggable={false}
          src={`https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/${zoom}/${ty}/${((tx % max) + max) % max}`}
          className="absolute h-64 w-64 max-w-none"
          style={{ left, top }}
        />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// (26/10) Fotos en caché + precarga (el mapa las pide para todos sus
// aeropuertos, así la tarjeta abre con la foto ya lista).
// ---------------------------------------------------------------------------

/** URL de la foto, `null` si no tiene, `undefined` mientras se busca. */
type PhotoState = string | null | undefined;

/** ICAO → URL de la foto (ya convertida) o `null` si no tiene. */
const photoCache = new Map<string, string | null>();
const photoInflight = new Map<string, Promise<string | null>>();

function photoStateOf(icao: string): PhotoState {
  const k = icao.toUpperCase();
  return photoCache.has(k) ? photoCache.get(k) ?? null : undefined;
}

/** Foto del aeropuerto (de la caché si ya se pidió). Deja la imagen decodificada. */
export function airportPhotoUrl(icao: string, name: string | null): Promise<string | null> {
  const k = icao.toUpperCase();
  if (photoCache.has(k)) return Promise.resolve(photoCache.get(k) ?? null);
  const running = photoInflight.get(k);
  if (running) return running;
  const p = api
    .airportPhoto(k, name)
    .then(async (path) => {
      const url = path && isTauri ? convertFileSrc(path) : null;
      if (url) {
        const img = new Image();
        img.src = url;
        await img.decode().catch(() => {});
      }
      photoCache.set(k, url);
      return url;
    })
    .catch(() => null) // error de red: no se cachea, se reintenta luego
    .finally(() => photoInflight.delete(k));
  photoInflight.set(k, p);
  return p;
}

const prefetchQueue: { icao: string; name: string | null }[] = [];
const prefetchSeen = new Set<string>();
let prefetchWorkers = 0;

/** Precarga (de dos en dos) las fotos de los aeropuertos del mapa. */
export function prefetchAirportPhotos(list: { icao: string; name: string | null }[]) {
  for (const a of list) {
    const k = a.icao?.toUpperCase();
    if (!k || prefetchSeen.has(k)) continue;
    prefetchSeen.add(k);
    prefetchQueue.push({ icao: k, name: a.name });
  }
  while (prefetchWorkers < 2 && prefetchQueue.length > 0) {
    prefetchWorkers++;
    void (async () => {
      for (let next = prefetchQueue.shift(); next; next = prefetchQueue.shift()) {
        await airportPhotoUrl(next.icao, next.name);
      }
      prefetchWorkers--;
    })();
  }
}

// ---------------------------------------------------------------------------

function Tile({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div title={title} className="flex min-h-[52px] items-center gap-2 rounded-lg bg-sf-raised/70 px-2.5 py-1.5">
      {children}
    </div>
  );
}

function MoneyLine({ label, usd }: { label: string; usd: number }) {
  return (
    <>
      <span>{label}</span>
      <span className="text-right font-mono">{fmtUsd(usd)}</span>
    </>
  );
}

const normReg = (s: string) => s.replace(/[^a-z0-9]/gi, "").toUpperCase();

/** El clip del mejor aterrizaje ahí (por hora), o el más suave grabado ahí.
 *  (26/10) Con filtro de aerolínea, sólo los de sus aviones (`regs`). */
function pickClip(clips: LandingClip[], d: AirportCardData, regs: Set<string> | null): LandingClip | null {
  const here = clips.filter(
    (c) =>
      !c.isTest &&
      c.airportIcao?.toUpperCase() === d.icao.toUpperCase() &&
      (!regs || (c.registration != null && regs.has(normReg(c.registration)))),
  );
  if (here.length === 0) return null;
  const best = d.bestLandingAt ? Date.parse(d.bestLandingAt) : NaN;
  if (!Number.isNaN(best)) {
    const near = here.find((c) => Math.abs(Date.parse(c.recordedAt) - best) < 30 * 60_000);
    if (near) return near;
  }
  const soft = (c: LandingClip) => (c.fpm == null ? -Infinity : -Math.abs(c.fpm));
  return [...here].sort((a, b) => soft(b) - soft(a))[0];
}

/** Rumbo verdadero (0–360) de `a` a `b`, ambos `[lat, lon]`. */
function bearing(a: [number, number], b: [number, number]): number {
  const r = Math.PI / 180;
  const [φ1, φ2, Δλ] = [a[0] * r, b[0] * r, (b[1] - a[1]) * r];
  const y = Math.sin(Δλ) * Math.cos(φ2);
  const x = Math.cos(φ1) * Math.sin(φ2) - Math.sin(φ1) * Math.cos(φ2) * Math.cos(Δλ);
  return ((Math.atan2(y, x) / r) + 360) % 360;
}

/** Superficie de OurAirports ("ASP", "CON", "TURF-G"…) en palabras. */
function surfaceLabel(s: string | null): string | null {
  if (!s) return null;
  const u = s.toUpperCase();
  const key = /ASP|BIT|TAR|BLACKTOP/.test(u)
    ? "asphalt"
    : /CON|PEM|CEM/.test(u)
      ? "concrete"
      : /GRS|GRASS|TURF/.test(u)
        ? "grass"
        : /GRV|GRE|GRAVEL/.test(u)
          ? "gravel"
          : /DIRT|CLA|SAND|EARTH|SOIL/.test(u)
            ? "dirt"
            : /WAT/.test(u)
              ? "water"
              : /SNOW|ICE/.test(u)
                ? "snow"
                : null;
  return key ? t(`fb.airport.surface.${key}`) : s;
}

function fpmColor(fpm: number): string {
  return fpm > -100 ? "text-cyan-300" : fpm > -300 ? "text-emerald-300" : fpm > -400 ? "text-amber-300" : "text-rose-400";
}

function fmtFpm(fpm: number): string {
  return `${fpm < 0 ? "−" : ""}${Math.abs(fpm)}`;
}

function fmtInt(n: number): string {
  return new Intl.NumberFormat(getActiveLocale()).format(n);
}

function fmtUsd(n: number): string {
  return new Intl.NumberFormat(getActiveLocale(), {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0,
  }).format(n);
}

function fmtDay(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleDateString(getActiveLocale(), { day: "numeric", month: "short", year: "numeric" });
}

let REGION_DN: Intl.DisplayNames | null | undefined;
function countryName(iso?: string | null): string | null {
  if (!iso) return null;
  try {
    if (REGION_DN === undefined) {
      REGION_DN = new Intl.DisplayNames([getActiveLocale()], { type: "region" });
    }
    return REGION_DN?.of(iso.toUpperCase()) ?? iso;
  } catch {
    return iso;
  }
}
