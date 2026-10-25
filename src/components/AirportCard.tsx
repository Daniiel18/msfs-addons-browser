import { useEffect, useState, type ReactNode } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  ArrowDown,
  ArrowUp,
  ImageOff,
  Loader2,
  Plane,
  PlaneLanding,
  PlaneTakeoff,
  Play,
  Users,
  X,
} from "lucide-react";
import { api, isTauri } from "../lib/tauri";
import { getActiveLocale, t } from "../lib/i18n";
import type { AirportCardData, LandingClip } from "../lib/types";

/**
 * (24/10) Tarjeta de aeropuerto dentro del mapa del FlightBook (clic en un
 * aeropuerto). Despegues y aterrizajes con iconos, pasajeros que subieron y
 * bajaron, mejor aterrizaje (con su clip), tus aviones aparcados ahí, dinero
 * gastado ahí (GSX y tasas), gate más usado, ruta más volada y última visita.
 */
export function AirportCard({
  icao,
  name: hintName,
  onClose,
}: {
  icao: string;
  name?: string | null;
  onClose: () => void;
}) {
  const [data, setData] = useState<AirportCardData | null>(null);
  const [error, setError] = useState(false);
  const [photo, setPhoto] = useState<string | null>(null);
  const [clip, setClip] = useState<LandingClip | null>(null);

  useEffect(() => {
    let alive = true;
    setData(null);
    setError(false);
    setPhoto(null);
    setClip(null);
    api
      .airportCard(icao)
      .then((d) => {
        if (!alive) return;
        setData(d);
        void api
          .airportPhoto(icao, d.name ?? hintName ?? null)
          .then((p) => alive && p && isTauri && setPhoto(convertFileSrc(p)))
          .catch(() => {});
        void api
          .listLandingClips()
          .then((clips) => alive && setClip(pickClip(clips, d)))
          .catch(() => {});
      })
      .catch(() => alive && setError(true));
    return () => {
      alive = false;
    };
  }, [icao, hintName]);

  const title = data?.name ?? hintName ?? icao;
  const place = [data?.municipality, countryName(data?.isoCountry)].filter(Boolean).join(", ");
  const flights = (data?.departures ?? 0) + (data?.arrivals ?? 0);

  return (
    <div
      className="pointer-events-auto absolute bottom-3 right-3 top-12 z-20 flex w-[320px] max-w-[calc(100%-1.5rem)] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface/95 text-[13px] text-sf-text shadow-2xl backdrop-blur"
      onMouseDown={(e) => e.stopPropagation()}
    >
      {/* Foto */}
      <div className="relative h-28 shrink-0 overflow-hidden bg-gradient-to-br from-sky-900/60 via-slate-900 to-slate-950">
        {photo ? (
          <img src={photo} alt="" className="absolute inset-0 h-full w-full object-cover" />
        ) : (
          <div className="absolute inset-0 flex items-center justify-center text-sf-muted/60">
            <ImageOff className="h-6 w-6" />
          </div>
        )}
        <div className="absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-slate-950/80 to-transparent" />
        <span className="absolute bottom-2 left-3 font-mono text-xs font-semibold text-emerald-300">{icao}</span>
        <button
          onClick={onClose}
          title={t("fb.airport.close")}
          className="absolute right-2 top-2 rounded-md bg-slate-950/70 p-1 text-sf-text hover:bg-slate-950"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3 pt-2.5">
        <div className="text-[15px] font-medium leading-tight">{title}</div>
        <div className="text-xs text-sf-muted">
          <span className="font-mono">{icao}</span>
          {place && <> · {place}</>}
        </div>

        {error && <p className="mt-3 text-xs text-rose-300">{t("fb.airport.error")}</p>}
        {!data && !error && (
          <div className="flex justify-center py-8 text-sf-muted">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        )}

        {data && (
          <>
            {data.parked.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-1.5">
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
              <p className="mt-3 text-xs text-sf-muted">{t("fb.airport.no_flights")}</p>
            ) : (
              <>
                <div className="mt-2.5 grid grid-cols-2 gap-1.5">
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
                    <span className="mt-0.5 border-t border-sf-border pt-0.5 font-medium">{t("fb.airport.total")}</span>
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
                        {data.topGate}{" "}
                        <span className="text-sf-muted">({t("fb.airport.times", { n: String(data.topGateCount) })})</span>
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
    </div>
  );
}

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

/** El clip del mejor aterrizaje ahí (por hora), o el más suave grabado ahí. */
function pickClip(clips: LandingClip[], d: AirportCardData): LandingClip | null {
  const here = clips.filter((c) => !c.isTest && c.airportIcao?.toUpperCase() === d.icao.toUpperCase());
  if (here.length === 0) return null;
  const best = d.bestLandingAt ? Date.parse(d.bestLandingAt) : NaN;
  if (!Number.isNaN(best)) {
    const near = here.find((c) => Math.abs(Date.parse(c.recordedAt) - best) < 30 * 60_000);
    if (near) return near;
  }
  const soft = (c: LandingClip) => (c.fpm == null ? -Infinity : -Math.abs(c.fpm));
  return [...here].sort((a, b) => soft(b) - soft(a))[0];
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
