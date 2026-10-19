import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  BookOpen,
  CheckCircle2,
  ChevronRight,
  ClipboardCheck,
  Compass,
  HardDrive,
  HeartPulse,
  Layers,
  Loader2,
  Navigation,
  Package,
  Plane,
  PlaneTakeoff,
  Rocket,
  Sparkles,
  Trash2,
  Trophy,
  Truck,
  Unlink,
  Users,
  X,
} from "lucide-react";
import type {
  AddonHealth,
  DashboardStats,
  FlightLogEntry,
  PilotProfile,
} from "../lib/types";
import { api } from "../lib/tauri";
import { useAppStore } from "../stores/useAppStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import {
  useGsxUpdateStore,
  gsxUpdateVisible,
} from "../stores/useGsxUpdateStore";
import {
  useAiracUpdateStore,
  airacUpdateVisible,
} from "../stores/useAiracUpdateStore";
import { useNewSceneryStore } from "../stores/useNewSceneryStore";
import { useToastStore } from "../stores/useToastStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { usePreflight } from "../lib/usePreflight";
import { usePreflightStore } from "../stores/usePreflightStore";
import { useUnits } from "../lib/units";
import { cleanAtcType } from "../lib/aircraft";
import { DiskCleanupModal } from "./DiskCleanupModal";
import { ShareCardModal } from "./ShareCardModal";
import { cleanAircraftLabel } from "./FlightBookView";
import { effectiveRoute, phaseDisplay } from "./FlyingNowBadge";
import { computeWrapped } from "../lib/wrapped";
import { buildWrappedSvg } from "../lib/shareCards";
import { getActiveLocale } from "../lib/i18n";
import { derivedType } from "../lib/packageType";
import { cn } from "../lib/cn";
import { t } from "../lib/i18n";

const RADAR_THROTTLE_MS = 30 * 60 * 1000;
const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
/** Un OFP de SimBrief cuenta como "próximo vuelo" durante 36 h. */
const NEXT_OFP_MAX_AGE_MS = 36 * 60 * 60 * 1000;

/**
 * Inicio (antes "Dashboard de Community").
 *
 * (Rediseño F2) Estructura de arriba abajo:
 *   1. Bienvenida + tarjeta de vuelo (en curso → próximo OFP → último).
 *   2. Accesos rápidos (Explorar, Instalados, FlightBook, Preparar vuelo).
 *   3. KPIs: addons instalados, espacio ocupado, updates, vuelos.
 *   4. Últimos vuelos + Avisos y actualizaciones (GSX, AIRAC, radar de
 *      escenarios, pre-vuelo, salud, Wrapped) — antes eran banners
 *      apilados a todo el ancho.
 *   5. Datos de Community: desglose por tipo, top desarrolladores, más
 *      pesados, añadidos recientemente y centro de salud.
 */
export function DashboardView() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [health, setHealth] = useState<AddonHealth | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const setView = useAppStore((s) => s.setView);
  const healthRef = useRef<HTMLDivElement | null>(null);

  // (v6.2.4) Aviso de update de GSX (FSDreamTeam) — el chequeo horario vive en
  // App.tsx; aquí leemos el resultado compartido.
  const gsxInfo = useGsxUpdateStore((s) => s.info);
  const gsxRequestInstall = useGsxUpdateStore((s) => s.requestInstall);
  const showGsx = gsxUpdateVisible(gsxInfo);
  // (v6.2.8) Aviso de update de AIRAC.
  const airacInfo = useAiracUpdateStore((s) => s.info);
  const airacOpenUpdater = useAiracUpdateStore((s) => s.openUpdater);
  const showAirac = airacUpdateVisible(airacInfo);

  // Recalculamos automáticamente cuando termina un scan de Community
  // — el usuario no necesita pulsar "refrescar". `scanning` flipa
  // false → true → false en cada scan; observamos la transición a
  // false para recargar las stats.
  const scanning = useCommunityStore((s) => s.scanning);
  const packages = useCommunityStore((s) => s.packages);
  const packagesCount = packages.length;

  // (v6.2.26 / R3, v6.2.34) Chequeo pre-vuelo "¿Listo para volar?" +
  // radar. El resultado es compartido (usePreflight) para que el badge
  // del Dashboard y la campanita coincidan; el modal se abre a nivel App
  // vía usePreflightStore.
  const [showCleanup, setShowCleanup] = useState(false);
  const [showWrapped, setShowWrapped] = useState(false);
  const [pilot, setPilot] = useState<PilotProfile | null>(null);
  const simVersion = useSettingsStore((s) => s.settings.simVersion);
  const logEntries = useFlightLogStore((s) => s.entries);
  const freshScenery = useNewSceneryStore((s) => s.fresh);
  const radarCheck = useNewSceneryStore((s) => s.check);
  const radarLastChecked = useNewSceneryStore((s) => s.lastCheckedAt);
  const openPreflight = usePreflightStore((s) => s.setOpen);
  const preflightIssues = usePreflight().result.issues;

  // Radar: navega el catálogo al montar el Dashboard, con throttle para
  // no golpear la red al ir y volver de vista.
  useEffect(() => {
    if (radarLastChecked && Date.now() - radarLastChecked < RADAR_THROTTLE_MS) {
      return;
    }
    void radarCheck(simVersion);
  }, [radarCheck, simVersion, radarLastChecked]);

  const preflightBadge = preflightIssues + freshScenery.length;

  // (v6.2.30 / R7) Wrapped anual — perfil del piloto para personalizar
  // la tarjeta + tarjeta SVG lista para compartir.
  useEffect(() => {
    api.pilotProfile().then(setPilot).catch(() => {});
  }, []);
  // (v6.2.33) El Wrapped es del AÑO YA TERMINADO (el natural que pasó),
  // no del año en curso — el usuario no lo quiere "siempre presente".
  // Se muestra sólo si hay vuelos de ese año y hasta que se descarta.
  const wrappedYear = new Date().getFullYear() - 1;
  const wrappedDismissKey = `simfleet.wrapped.dismissed.${wrappedYear}`;
  const [wrappedDismissed, setWrappedDismissed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(wrappedDismissKey) === "1";
    } catch {
      return false;
    }
  });
  const dismissWrapped = () => {
    try {
      localStorage.setItem(wrappedDismissKey, "1");
    } catch {
      /* ignore */
    }
    setWrappedDismissed(true);
  };
  const wrappedCard = useMemo(() => {
    const stats = computeWrapped(logEntries, wrappedYear);
    const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
    return buildWrappedSvg(
      stats,
      {
        title: `Wrapped ${wrappedYear}`,
        pilot: pilot?.name ?? "",
        flights: t("wrapped.flights"),
        hours: t("wrapped.hours"),
        distance: t("wrapped.distance"),
        airports: t("wrapped.airports"),
        topDestinations: t("wrapped.top_destinations"),
        topAircraft: t("wrapped.top_aircraft"),
        bestLanding: t("wrapped.best_landing"),
        tagline: t("wrapped.tagline"),
      },
      locale,
    );
  }, [logEntries, wrappedYear, pilot]);
  const wrappedFlights = useMemo(
    () => computeWrapped(logEntries, wrappedYear).flights,
    [logEntries, wrappedYear],
  );

  // (v6 hotfix) Conteo de aviones/liveries con el MISMO clasificador
  // (`derivedType`) que usan Addons y el Mapa. El backend de `stats` los
  // contaba crudo (solo content_type=="AIRCRAFT") y subcontaba: liveries con
  // manifest mínimo o aviones de 3rd-party caían en "Otros". Así el KPI del
  // Dashboard coincide con la pestaña Addons.
  const { aircraftCount, liveriesCount } = useMemo(() => {
    let ac = 0;
    let lv = 0;
    for (const p of packages) {
      const d = derivedType(p);
      if (d === "AIRCRAFT") ac += 1;
      else if (d === "LIVERY") lv += 1;
    }
    return { aircraftCount: ac, liveriesCount: lv };
  }, [packages]);

  // (Rediseño F2) Paquetes cuya carpeta cambió en los últimos 7 días
  // (instalados o actualizados esta semana).
  const addonsThisWeek = useMemo(() => {
    const now = Date.now();
    return packages.filter((p) => {
      const ts = p.folderModifiedAt ? Date.parse(p.folderModifiedAt) : NaN;
      return Number.isFinite(ts) && now - ts < WEEK_MS;
    }).length;
  }, [packages]);

  // (Rediseño F2) Vuelos completados (los que muestra el FlightBook),
  // del más reciente al más antiguo.
  const completedFlights = useMemo(
    () =>
      logEntries
        .filter((e) => e.status === "completed")
        .sort((a, b) => Date.parse(b.startedAt) - Date.parse(a.startedAt)),
    [logEntries],
  );
  const flightsThisMonth = useMemo(() => {
    const now = new Date();
    return completedFlights.filter((e) => {
      const d = new Date(e.startedAt);
      return d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth();
    }).length;
  }, [completedFlights]);

  const load = () => {
    setLoading(true);
    setError(null);
    Promise.all([api.getDashboardStats(), api.getAddonHealth()])
      .then(([s, h]) => {
        setStats(s);
        setHealth(h);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    load();
  }, []);

  // Recarga cuando termina un scan o cambia el inventario.
  useEffect(() => {
    if (!scanning) load();
  }, [scanning, packagesCount]);

  // (Rediseño F2) Avisos que antes eran banners sueltos. Cada uno conserva
  // la misma acción que tenía.
  const notices: Notice[] = [];
  if (stats && stats.updatesAvailable > 0) {
    notices.push({
      key: "updates",
      icon: <Sparkles className="h-4 w-4" />,
      tone: "amber",
      title: t("home.notices.addon_updates"),
      desc: t("home.notices.addon_updates_desc", { n: String(stats.updatesAvailable) }),
      onClick: () => window.dispatchEvent(new Event("simfleet:notifications")),
    });
  }
  if (showGsx && gsxInfo) {
    notices.push({
      key: "gsx",
      icon: <Truck className="h-4 w-4" />,
      tone: "cyan",
      title: t("notifications.gsx_update"),
      desc: `${gsxInfo.installedVersion} → ${gsxInfo.latestVersion}`,
      meta: gsxInfo.latestDate ?? undefined,
      onClick: () => gsxRequestInstall(),
    });
  }
  if (showAirac && airacInfo) {
    notices.push({
      key: "airac",
      icon: <Navigation className="h-4 w-4" />,
      tone: "indigo",
      title: t("notifications.airac_update"),
      desc: `${t("notifications.airac_cycle")} ${airacInfo.installedCycle} → ${airacInfo.latestCycle}`,
      meta: airacInfo.effectiveDate ?? undefined,
      onClick: () => void airacOpenUpdater(),
    });
  }
  if (freshScenery.length > 0) {
    notices.push({
      key: "fresh",
      icon: <Package className="h-4 w-4" />,
      tone: "brand",
      title: t("home.notices.fresh"),
      desc: t("preflight.cta.fresh", { n: String(freshScenery.length) }),
      onClick: () => openPreflight(true),
    });
  }
  if (preflightIssues > 0) {
    notices.push({
      key: "preflight",
      icon: <ClipboardCheck className="h-4 w-4" />,
      tone: "amber",
      title: t("home.notices.preflight"),
      desc: t("preflight.cta.issues", { n: String(preflightIssues) }),
      onClick: () => openPreflight(true),
    });
  }
  if (health && health.issueCount > 0) {
    notices.push({
      key: "health",
      icon: <HeartPulse className="h-4 w-4" />,
      tone: "rose",
      title: t("home.notices.health"),
      desc: t("dashboard.health.to_review", { n: String(health.issueCount) }),
      onClick: () =>
        healthRef.current?.scrollIntoView({ behavior: "smooth", block: "start" }),
    });
  }
  if (wrappedFlights > 0 && !wrappedDismissed) {
    notices.push({
      key: "wrapped",
      icon: <Sparkles className="h-4 w-4" />,
      tone: "fuchsia",
      title: t("wrapped.cta.title", { year: String(wrappedYear) }),
      desc: t("wrapped.cta.hint", { n: String(wrappedFlights) }),
      onClick: () => setShowWrapped(true),
      onDismiss: dismissWrapped,
    });
  }

  return (
    <div className="space-y-4 pb-2">
      <WelcomeHero pilotName={pilot?.name ?? null} lastFlight={completedFlights[0] ?? null} />

      <QuickActions
        preflightBadge={preflightBadge}
        preflightWarn={preflightIssues > 0}
        onExplore={() => setView("search")}
        onInstalled={() => setView("addons")}
        onFlightBook={() => setView("flightbook")}
        onPreflight={() => openPreflight(true)}
      />

      {showCleanup && (
        <DiskCleanupModal onClose={() => setShowCleanup(false)} />
      )}
      {showWrapped && (
        <ShareCardModal
          title={t("wrapped.cta.title", { year: String(wrappedYear) })}
          card={wrappedCard}
          onClose={() => setShowWrapped(false)}
        />
      )}

      {error && (
        <div className="flex items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
          <AlertCircle className="h-3.5 w-3.5" />
          <span>{error}</span>
        </div>
      )}

      {/* KPIs — 4 columnas en xl, 2 en sm. */}
      {stats && (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard
            icon={<Package className="h-5 w-5" />}
            label={t("home.kpi.addons")}
            value={stats.totalPackages.toLocaleString("es-ES")}
            hint={
              addonsThisWeek > 0
                ? t("home.kpi.addons_week", { n: String(addonsThisWeek) })
                : t("dashboard.kpi.airports.hint", { aircraft: aircraftCount, liveries: liveriesCount })
            }
            hintTone={addonsThisWeek > 0 ? "positive" : "muted"}
          />
          <button
            onClick={() => setShowCleanup(true)}
            title={t("cleanup.cta")}
            className="rounded-xl text-left outline-none transition-transform hover:-translate-y-0.5 focus-visible:ring-2 focus-visible:ring-brand-500/60"
          >
            <StatCard
              icon={<HardDrive className="h-5 w-5" />}
              // (v7.4.2) Es lo que OCUPAN los addons de la Community, no el
              // espacio libre del disco.
              label={t("home.kpi.disk")}
              value={formatBytes(stats.totalSizeBytes)}
              hint={t("home.kpi.disk_hint")}
            />
          </button>
          <StatCard
            icon={<Sparkles className="h-5 w-5" />}
            label={t("home.kpi.updates")}
            value={stats.updatesAvailable.toLocaleString("es-ES")}
            valueTone={stats.updatesAvailable > 0 ? "warning" : undefined}
            hint={
              stats.updatesAvailable === 0
                ? t("dashboard.kpi.updates.all_clear")
                : t("home.kpi.updates_pending")
            }
            hintTone={stats.updatesAvailable > 0 ? "warning" : "muted"}
          />
          <StatCard
            icon={<PlaneTakeoff className="h-5 w-5" />}
            label={t("home.kpi.flights")}
            value={completedFlights.length.toLocaleString("es-ES")}
            hint={t("home.kpi.flights_month", { n: String(flightsThisMonth) })}
          />
        </div>
      )}

      {!stats && loading && (
        <div className="flex items-center justify-center rounded-xl border border-sf-border/80 bg-sf-surface px-4 py-12 text-sm text-sf-muted">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("dashboard.computing")}
        </div>
      )}

      <div className="grid items-stretch gap-3 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <RecentFlights
          entries={completedFlights.slice(0, 5)}
          onOpenFlightBook={() => setView("flightbook")}
        />
        <NoticesPanel notices={notices} />
      </div>

      {stats && stats.totalPackages === 0 && !loading && (
        <div className="rounded-xl border border-sf-border/80 bg-sf-surface px-4 py-12 text-center text-sm text-sf-muted">
          <Package className="mx-auto mb-3 h-8 w-8 text-sf-muted/60" />
          <p>{t("dashboard.empty.title")}</p>
          <p className="mt-1 text-xs text-sf-muted/80">
            {t("dashboard.empty.hint")}
          </p>
        </div>
      )}

      {stats && stats.totalPackages > 0 && (
        <>
          <Card title={t("dashboard.card.by_type")} icon={<Rocket className="h-4 w-4" />}>
            <TypeBreakdown stats={stats} />
          </Card>

          {/* Top devs y largest packages se estiran a la misma
              altura usando `items-stretch`, y dentro las cards crecen
              con `h-full + flex-col`. */}
          <div className="grid items-stretch gap-3 lg:grid-cols-2">
            <Card
              title={t("dashboard.card.top_devs")}
              icon={<Users className="h-4 w-4" />}
              hint={t("dashboard.card.top_devs.hint")}
            >
              {stats.topCreators.length === 0 ? (
                <p className="px-1 py-2 text-xs text-sf-muted">
                  {t("dashboard.card.top_devs.empty")}
                </p>
              ) : (
                <CreatorsTable creators={stats.topCreators} />
              )}
            </Card>

            <Card
              title={t("dashboard.card.largest")}
              icon={<Trophy className="h-4 w-4" />}
              hint={t("dashboard.card.largest.hint")}
            >
              {stats.largestPackages.length === 0 ? (
                <p className="px-1 py-2 text-xs text-sf-muted">
                  {t("dashboard.card.largest.empty")}
                </p>
              ) : (
                <LargestTable
                  rows={stats.largestPackages}
                  maxBytes={stats.largestPackages[0].sizeBytes ?? 1}
                />
              )}
            </Card>
          </div>

          {stats.recentlyAdded.length > 0 && (
            <Card title={t("dashboard.card.recently_added")} icon={<Plane className="h-4 w-4" />}>
              <ul className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
                {stats.recentlyAdded.map((p) => (
                  <li
                    key={p.folderName}
                    className="flex items-center gap-2.5 rounded-lg border border-sf-border/60 bg-sf-bg/40 px-3 py-2 text-[13px]"
                  >
                    <Package className="h-4 w-4 shrink-0 text-sf-muted" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sf-text">{p.title}</div>
                      <div className="truncate text-[11px] text-sf-muted">
                        {p.creator ?? t("dashboard.unknown_author")}
                        {p.sizeBytes != null && ` · ${formatBytes(p.sizeBytes)}`}
                      </div>
                    </div>
                  </li>
                ))}
              </ul>
            </Card>
          )}

          {health && (
            <div ref={healthRef} className="scroll-mt-4">
              <HealthCenter health={health} onChanged={load} />
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ----------------------------------------------------------------------------
// (Rediseño F2) Bienvenida + tarjeta de vuelo
// ----------------------------------------------------------------------------

function WelcomeHero({
  pilotName,
  lastFlight,
}: {
  pilotName: string | null;
  lastFlight: FlightLogEntry | null;
}) {
  return (
    <section className="relative overflow-hidden rounded-2xl border border-sf-border/80 bg-gradient-to-br from-sf-surface via-sf-surface to-brand-500/15">
      <RouteArt className="pointer-events-none absolute inset-y-0 right-0 h-full w-full text-brand-400/25 lg:w-3/4" />
      <div className="relative flex flex-col gap-5 p-6 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0">
          <h1 className="text-2xl font-semibold tracking-tight text-sf-text">
            {pilotName
              ? t("home.hero.welcome_named", { name: pilotName })
              : t("home.hero.welcome")}
          </h1>
          <p className="mt-1.5 max-w-xl text-sm text-sf-muted">
            {t("home.hero.subtitle")}
          </p>
        </div>
        <FlightCard lastFlight={lastFlight} />
      </div>
    </section>
  );
}

/** Trazos de rutas decorativos (SVG puro, sin imágenes). */
function RouteArt({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 640 220"
      preserveAspectRatio="xMaxYMid slice"
      className={className}
      fill="none"
      aria-hidden
    >
      <g stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeDasharray="2 7">
        <path d="M40 190 C 200 40, 420 30, 610 120" />
        <path d="M150 210 C 260 110, 430 90, 600 40" />
        <path d="M300 200 C 380 150, 500 150, 630 175" />
      </g>
      <g fill="currentColor">
        <circle cx="40" cy="190" r="3.5" />
        <circle cx="610" cy="120" r="3.5" />
        <circle cx="600" cy="40" r="3" />
        <circle cx="630" cy="175" r="3" />
      </g>
      <path
        d="M404 66 l18 -6 l-6 18 l-3 -7 z"
        fill="currentColor"
        opacity="0.9"
      />
    </svg>
  );
}

/**
 * Tarjeta de vuelo del hero, por prioridad:
 *   1. Vuelo en curso (SimConnect conectado).
 *   2. Próximo vuelo: el OFP de SimBrief más reciente (< 36 h).
 *   3. Último vuelo completado del FlightBook.
 * Sin ninguno de los tres no se muestra.
 */
function FlightCard({ lastFlight }: { lastFlight: FlightLogEntry | null }) {
  const status = useFlightLogStore((s) => s.status);
  const ofps = useSimBriefStore((s) => s.flights);
  const setView = useAppStore((s) => s.setView);
  const u = useUnits();

  const latestOfp = useMemo(() => {
    let best: (typeof ofps)[number] | null = null;
    let bestTs = 0;
    for (const f of ofps) {
      const ts = f.generatedAt ? parseInt(f.generatedAt, 10) * 1000 : 0;
      if (ts > bestTs) {
        best = f;
        bestTs = ts;
      }
    }
    return best && Date.now() - bestTs < NEXT_OFP_MAX_AGE_MS ? best : null;
  }, [ofps]);

  let kind: "live" | "next" | "last";
  let origin: string | null;
  let destination: string | null;
  let originName: string | null = null;
  let destinationName: string | null = null;
  let pill: { label: string; dot: string } | null = null;
  let cells: { label: string; value: string }[];

  const live =
    !!status?.simRunning &&
    status.simconnectConnected &&
    !!(status.currentAirportIcao || status.originIcao);

  if (live && status) {
    const r = effectiveRoute(status, ofps);
    const phase = phaseDisplay(status.phaseLabel);
    kind = "live";
    origin = r.effectiveOrigin;
    destination = r.effectiveDestination;
    originName = status.originName ?? r.matchedOfp?.originName ?? null;
    destinationName = status.destinationName ?? r.matchedOfp?.destinationName ?? null;
    pill = { label: phase.short, dot: phase.dotClass };
    cells = [
      { label: t("home.flight.altitude"), value: u.fmt.altitude(status.currentAltFt) },
      { label: t("home.flight.speed"), value: u.fmt.speed(status.currentGroundSpeedKt) },
      { label: t("home.flight.distance"), value: u.fmt.distance(status.distanceNm ?? r.matchedOfp?.distanceNm ?? null) },
    ];
  } else if (latestOfp) {
    kind = "next";
    origin = latestOfp.originIcao;
    destination = latestOfp.destinationIcao;
    originName = latestOfp.originName;
    destinationName = latestOfp.destinationName;
    pill = { label: "SimBrief", dot: "bg-sky-400" };
    cells = [
      { label: t("home.flight.aircraft"), value: latestOfp.aircraftIcao ?? "—" },
      { label: t("home.flight.distance"), value: u.fmt.distance(latestOfp.distanceNm) },
      { label: t("home.flight.ete"), value: formatDuration(latestOfp.estTimeEnrouteS) },
    ];
  } else if (lastFlight) {
    kind = "last";
    origin = lastFlight.originIcao;
    destination = lastFlight.destinationIcao;
    originName = lastFlight.originName;
    destinationName = lastFlight.destinationName;
    pill = { label: formatShortDate(lastFlight.startedAt), dot: "bg-slate-400" };
    cells = [
      { label: t("home.flight.aircraft"), value: aircraftLabel(lastFlight) },
      { label: t("home.flight.duration"), value: formatDuration(lastFlight.flightTimeS) },
      { label: t("home.flight.landing"), value: u.fmt.vs(lastFlight.landingFpm) },
    ];
  } else {
    return null;
  }

  const title =
    kind === "live"
      ? t("home.flight.current")
      : kind === "next"
        ? t("home.flight.next")
        : t("home.flight.last");

  return (
    <button
      onClick={() => setView("flightbook")}
      className="w-full shrink-0 rounded-xl border border-sf-border bg-sf-bg/70 p-4 text-left outline-none transition-colors hover:border-brand-500/40 focus-visible:ring-2 focus-visible:ring-brand-500/60 lg:w-[22rem]"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-sf-muted">{title}</span>
        {pill && (
          <span className="inline-flex min-w-0 items-center gap-1.5 truncate text-xs text-sf-muted">
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", pill.dot)} />
            {pill.label}
          </span>
        )}
      </div>
      <div className="mt-2 flex items-center gap-2 font-mono text-xl font-semibold text-sf-text">
        <span>{origin ?? "····"}</span>
        <span className="text-brand-300">→</span>
        <span>{destination ?? "····"}</span>
      </div>
      {(originName || destinationName) && (
        <div className="mt-0.5 truncate text-xs text-sf-muted">
          {originName ?? "—"} → {destinationName ?? "—"}
        </div>
      )}
      <div className="mt-3 grid grid-cols-3 gap-2 border-t border-sf-border/70 pt-3">
        {cells.map((c) => (
          <div key={c.label} className="min-w-0">
            <div className="truncate text-[11px] uppercase tracking-wide text-sf-muted">
              {c.label}
            </div>
            <div className="mt-0.5 truncate text-sm font-semibold tabular-nums text-sf-text">
              {c.value}
            </div>
          </div>
        ))}
      </div>
    </button>
  );
}

// ----------------------------------------------------------------------------
// (Rediseño F2) Accesos rápidos
// ----------------------------------------------------------------------------

function QuickActions({
  preflightBadge,
  preflightWarn,
  onExplore,
  onInstalled,
  onFlightBook,
  onPreflight,
}: {
  preflightBadge: number;
  preflightWarn: boolean;
  onExplore: () => void;
  onInstalled: () => void;
  onFlightBook: () => void;
  onPreflight: () => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <QuickAction
        primary
        icon={<Compass className="h-5 w-5" />}
        title={t("home.quick.explore.title")}
        desc={t("home.quick.explore.desc")}
        onClick={onExplore}
      />
      <QuickAction
        icon={<Package className="h-5 w-5" />}
        title={t("home.quick.installed.title")}
        desc={t("home.quick.installed.desc")}
        onClick={onInstalled}
      />
      <QuickAction
        icon={<BookOpen className="h-5 w-5" />}
        title={t("home.quick.flightbook.title")}
        desc={t("home.quick.flightbook.desc")}
        onClick={onFlightBook}
      />
      <QuickAction
        icon={<ClipboardCheck className="h-5 w-5" />}
        title={t("home.quick.preflight.title")}
        desc={t("home.quick.preflight.desc")}
        onClick={onPreflight}
        badge={preflightBadge > 0 ? preflightBadge : undefined}
        badgeWarn={preflightWarn}
      />
    </div>
  );
}

function QuickAction({
  icon,
  title,
  desc,
  onClick,
  primary,
  badge,
  badgeWarn,
}: {
  icon: React.ReactNode;
  title: string;
  desc: string;
  onClick: () => void;
  primary?: boolean;
  badge?: number;
  badgeWarn?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "group relative flex items-start gap-3 rounded-xl border p-4 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-brand-500/60",
        primary
          ? "border-brand-400/30 bg-gradient-to-br from-brand-600 to-brand-800 text-white hover:from-brand-500 hover:to-brand-700"
          : "border-sf-border/80 bg-sf-surface hover:border-brand-500/40 hover:bg-sf-raised",
      )}
    >
      <div
        className={cn(
          "flex h-10 w-10 shrink-0 items-center justify-center rounded-xl",
          primary ? "bg-white/15 text-white" : "bg-sf-raised text-brand-300",
        )}
      >
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className={cn("text-sm font-semibold", primary ? "text-white" : "text-sf-text")}>
          {title}
        </div>
        <div className={cn("mt-0.5 text-xs leading-snug", primary ? "text-white/80" : "text-sf-muted")}>
          {desc}
        </div>
      </div>
      {badge !== undefined && (
        <span
          className={cn(
            "absolute right-3 top-3 inline-flex min-w-5 items-center justify-center rounded-full px-1.5 py-0.5 text-[11px] font-bold",
            badgeWarn ? "bg-amber-500/20 text-amber-300" : "bg-brand-500/20 text-brand-200",
          )}
        >
          {badge}
        </span>
      )}
    </button>
  );
}

// ----------------------------------------------------------------------------
// (Rediseño F2) Últimos vuelos
// ----------------------------------------------------------------------------

function RecentFlights({
  entries,
  onOpenFlightBook,
}: {
  entries: FlightLogEntry[];
  onOpenFlightBook: () => void;
}) {
  const u = useUnits();
  return (
    <Card
      title={t("home.recent.title")}
      icon={<BookOpen className="h-4 w-4" />}
      action={
        entries.length > 0 ? (
          <button
            onClick={onOpenFlightBook}
            className="inline-flex items-center gap-0.5 text-xs font-medium text-brand-300 hover:text-brand-200"
          >
            {t("home.recent.see_all")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        ) : undefined
      }
    >
      {entries.length === 0 ? (
        <p className="px-1 py-6 text-center text-sm text-sf-muted">
          {t("home.recent.empty")}
        </p>
      ) : (
        <div className="overflow-hidden rounded-lg border border-sf-border/60">
          <div className="grid grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,1.1fr)_4.5rem_6.5rem] items-center gap-3 border-b border-sf-border/60 bg-sf-raised/50 px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
            <span>{t("home.recent.col.date")}</span>
            <span>{t("home.recent.col.aircraft")}</span>
            <span>{t("home.recent.col.route")}</span>
            <span className="text-right">{t("home.recent.col.duration")}</span>
            <span className="text-right">{t("home.recent.col.result")}</span>
          </div>
          <ul className="divide-y divide-sf-border/50">
            {entries.map((e) => (
              <li key={e.id}>
                <button
                  onClick={onOpenFlightBook}
                  className="grid w-full grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,1.1fr)_4.5rem_6.5rem] items-center gap-3 px-3 py-2.5 text-left text-[13px] transition-colors hover:bg-sf-raised/60"
                >
                  <span className="text-sf-muted">{formatShortDate(e.startedAt)}</span>
                  <span className="truncate text-sf-text" title={e.aircraftTitle ?? undefined}>
                    {aircraftLabel(e)}
                  </span>
                  <span className="truncate font-mono text-xs text-sf-text">
                    {e.originIcao ?? "?"} → {e.destinationIcao ?? "?"}
                  </span>
                  <span className="text-right tabular-nums text-sf-muted">
                    {formatDuration(e.flightTimeS)}
                  </span>
                  <span className="flex items-center justify-end gap-1.5">
                    {e.landingFpm !== null && (
                      <span className="truncate text-xs tabular-nums text-sf-muted">
                        {u.fmt.vs(e.landingFpm)}
                      </span>
                    )}
                    {e.scoreGrade && (
                      <span
                        className={cn(
                          "shrink-0 rounded px-1.5 py-0.5 font-mono text-[11px] font-bold",
                          gradeClass(e.scoreGrade),
                        )}
                      >
                        {e.scoreGrade}
                      </span>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </Card>
  );
}

// ----------------------------------------------------------------------------
// (Rediseño F2) Avisos y actualizaciones
// ----------------------------------------------------------------------------

type NoticeTone = "brand" | "amber" | "cyan" | "indigo" | "rose" | "fuchsia";

interface Notice {
  key: string;
  icon: React.ReactNode;
  tone: NoticeTone;
  title: string;
  desc?: string;
  meta?: string;
  onClick?: () => void;
  onDismiss?: () => void;
}

const noticeTone: Record<NoticeTone, string> = {
  brand: "bg-brand-500/15 text-brand-300",
  amber: "bg-amber-500/15 text-amber-300",
  cyan: "bg-cyan-500/15 text-cyan-300",
  indigo: "bg-indigo-500/15 text-indigo-300",
  rose: "bg-rose-500/15 text-rose-300",
  fuchsia: "bg-fuchsia-500/15 text-fuchsia-300",
};

function NoticesPanel({ notices }: { notices: Notice[] }) {
  return (
    <Card title={t("home.notices.title")} icon={<Sparkles className="h-4 w-4" />}>
      {notices.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 py-6 text-center">
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-emerald-500/15">
            <CheckCircle2 className="h-5 w-5 text-emerald-300" />
          </div>
          <p className="text-sm text-sf-muted">{t("home.notices.empty")}</p>
        </div>
      ) : (
        <ul className="space-y-1">
          {notices.map((n) => (
            <li key={n.key} className="group flex items-center gap-1 rounded-lg hover:bg-sf-raised/60">
              <button
                onClick={n.onClick}
                className="flex min-w-0 flex-1 items-center gap-3 px-2 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-brand-500/60"
              >
                <span
                  className={cn(
                    "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg",
                    noticeTone[n.tone],
                  )}
                >
                  {n.icon}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium text-sf-text">
                    {n.title}
                  </span>
                  {n.desc && (
                    <span className="block truncate text-xs text-sf-muted">{n.desc}</span>
                  )}
                </span>
                {n.meta && (
                  <span className="shrink-0 text-[11px] text-sf-muted">{n.meta}</span>
                )}
                {!n.onDismiss && (
                  <ChevronRight className="h-4 w-4 shrink-0 text-sf-muted/70" />
                )}
              </button>
              {n.onDismiss && (
                <button
                  onClick={n.onDismiss}
                  title={t("common.dismiss")}
                  aria-label={t("common.dismiss")}
                  className="mr-1 shrink-0 rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                >
                  <X className="h-4 w-4" />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

// ----------------------------------------------------------------------------
// (Rediseño F2) Formato
// ----------------------------------------------------------------------------

function aircraftLabel(e: FlightLogEntry): string {
  return (
    cleanAircraftLabel(e.aircraftTitle) ||
    cleanAtcType(e.aircraftAtcType) ||
    e.aircraftModel ||
    "—"
  );
}

function formatShortDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
  return d.toLocaleDateString(locale, { day: "numeric", month: "short" });
}

function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return "—";
  const total = Math.round(seconds / 60);
  const h = Math.floor(total / 60);
  const m = total % 60;
  return h > 0 ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}m`;
}

/** Badge de calificación del vuelo — mismo código de color que el FlightBook. */
function gradeClass(grade: string): string {
  switch (grade) {
    case "A":
      return "bg-emerald-500/20 text-emerald-200 ring-1 ring-emerald-500/40";
    case "B":
      return "bg-sky-500/20 text-sky-200 ring-1 ring-sky-500/40";
    case "C":
      return "bg-amber-500/20 text-amber-200 ring-1 ring-amber-500/40";
    case "D":
      return "bg-orange-500/20 text-orange-200 ring-1 ring-orange-500/40";
    default:
      return "bg-rose-500/20 text-rose-200 ring-1 ring-rose-500/40";
  }
}

// ----------------------------------------------------------------------------
// (v7.3.0) Centro de salud de addons
// ----------------------------------------------------------------------------

type HealthTone = "rose" | "amber" | "violet";

const healthTone: Record<
  HealthTone,
  { tile: string; icon: string; badge: string }
> = {
  rose: {
    tile: "bg-rose-500/15",
    icon: "text-rose-300",
    badge: "bg-rose-500/15 text-rose-200",
  },
  amber: {
    tile: "bg-amber-500/15",
    icon: "text-amber-300",
    badge: "bg-amber-500/15 text-amber-200",
  },
  violet: {
    tile: "bg-violet-500/15",
    icon: "text-violet-300",
    badge: "bg-violet-500/15 text-violet-200",
  },
};

function HealthGroup({
  tone,
  icon,
  title,
  desc,
  count,
  children,
}: {
  tone: HealthTone;
  icon: React.ReactNode;
  title: string;
  desc?: string;
  count: number;
  children: React.ReactNode;
}) {
  const c = healthTone[tone];
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-2.5">
        <div
          className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${c.tile}`}
        >
          <span className={c.icon}>{icon}</span>
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-sf-text">{title}</span>
            <span
              className={`rounded-full px-1.5 text-[11px] font-medium ${c.badge}`}
            >
              {count}
            </span>
          </div>
          {desc && (
            <div className="truncate text-xs leading-snug text-sf-muted">
              {desc}
            </div>
          )}
        </div>
      </div>
      <ul className="space-y-1">{children}</ul>
    </div>
  );
}

function HealthRow({
  icon,
  title,
  subtitle,
  badge,
  tone,
  action,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
  badge?: string;
  tone: HealthTone;
  action?: React.ReactNode;
}) {
  const c = healthTone[tone];
  return (
    <li className="flex items-center gap-2.5 rounded-lg border border-sf-border/60 bg-sf-bg/40 px-3 py-2 text-[13px]">
      <span className={`shrink-0 ${c.icon}`}>{icon}</span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sf-text">{title}</div>
        {subtitle && (
          <div className="truncate text-[11px] text-sf-muted">{subtitle}</div>
        )}
      </div>
      {badge && (
        <span
          className={`shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium ${c.badge}`}
        >
          {badge}
        </span>
      )}
      {action && <div className="shrink-0">{action}</div>}
    </li>
  );
}

/** Botón de acción con confirmación inline (se transforma en ✓ / ✗). */
function RowAction({
  label,
  danger,
  busy,
  confirming,
  onAsk,
  onConfirm,
  onCancel,
}: {
  label: string;
  danger?: boolean;
  busy: boolean;
  confirming: boolean;
  onAsk: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  if (busy) {
    return <Loader2 className="h-3.5 w-3.5 animate-spin text-sf-muted" />;
  }
  if (confirming) {
    return (
      <div className="flex items-center gap-1">
        <button
          onClick={onConfirm}
          className="rounded border border-rose-500/40 bg-rose-500/15 px-2 py-0.5 text-[11px] font-medium text-rose-200 hover:bg-rose-500/25"
        >
          {t("dashboard.health.confirm")}
        </button>
        <button
          onClick={onCancel}
          className="rounded border border-sf-border px-2 py-0.5 text-[11px] text-sf-muted hover:bg-sf-raised"
        >
          {t("common.cancel")}
        </button>
      </div>
    );
  }
  return (
    <button
      onClick={onAsk}
      className={`rounded border px-2 py-0.5 text-[11px] font-medium ${
        danger
          ? "border-rose-500/30 text-rose-300 hover:bg-rose-500/15"
          : "border-sf-border text-sf-text hover:bg-sf-raised"
      }`}
    >
      {label}
    </button>
  );
}

function HealthCenter({
  health,
  onChanged,
}: {
  health: AddonHealth;
  onChanged: () => void;
}) {
  const { orphanedLiveries: orphans, simCompatMismatches: compat, artifacts } =
    health;
  const ok = health.issueCount === 0;
  const pushToast = useToastStore((s) => s.push);
  const [confirmKey, setConfirmKey] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const run = async (
    key: string,
    fn: () => Promise<unknown>,
    okMsg: string,
  ) => {
    setBusyKey(key);
    setConfirmKey(null);
    try {
      await fn();
      pushToast({ kind: "success", title: okMsg });
      onChanged();
    } catch (e) {
      pushToast({
        kind: "error",
        title: t("dashboard.health.action_failed"),
        message: String(e),
      });
    } finally {
      setBusyKey(null);
    }
  };

  const action = (
    key: string,
    label: string,
    danger: boolean,
    needsConfirm: boolean,
    exec: () => void,
  ) => (
    <RowAction
      label={label}
      danger={danger}
      busy={busyKey === key}
      confirming={confirmKey === key}
      onAsk={() => (needsConfirm ? setConfirmKey(key) : exec())}
      onConfirm={exec}
      onCancel={() => setConfirmKey(null)}
    />
  );

  return (
    <Card
      title={t("dashboard.health.title")}
      icon={<HeartPulse className="h-4 w-4" />}
    >
      {ok ? (
        <div className="flex flex-col items-center justify-center gap-2 rounded-xl border border-emerald-500/20 bg-emerald-500/[0.06] px-4 py-6 text-center">
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-emerald-500/15">
            <CheckCircle2 className="h-6 w-6 text-emerald-300" />
          </div>
          <div className="text-sm font-medium text-emerald-200">
            {t("dashboard.health.all_good")}
          </div>
        </div>
      ) : (
        <div className="space-y-3.5">
          {/* Franja resumen */}
          <div className="flex items-center gap-3 rounded-xl border border-rose-500/15 bg-rose-500/[0.06] px-3 py-2.5">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-rose-500/15">
              <AlertCircle className="h-5 w-5 text-rose-300" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-sf-text">
                {t("dashboard.health.to_review", { n: String(health.issueCount) })}
              </div>
              {health.reclaimableBytes > 0 && (
                <div className="text-xs text-sf-muted">
                  {formatBytes(health.reclaimableBytes)}{" "}
                  {t("dashboard.health.reclaimable")}
                </div>
              )}
            </div>
          </div>

          {orphans.length > 0 && (
            <HealthGroup
              tone="rose"
              icon={<Unlink className="h-4 w-4" />}
              title={t("dashboard.health.orphans_title")}
              desc={t("dashboard.health.orphans_desc")}
              count={orphans.length}
            >
              {orphans.map((o) => (
                <HealthRow
                  key={o.folderName}
                  tone="rose"
                  icon={<Plane className="h-3.5 w-3.5" />}
                  title={o.title || o.folderName}
                  subtitle={
                    o.base
                      ? `${t("dashboard.health.missing_base")}: ${o.base}`
                      : t("dashboard.health.missing_base")
                  }
                  badge={o.sizeBytes > 0 ? formatBytes(o.sizeBytes) : undefined}
                  action={action(
                    `orphan:${o.folderName}`,
                    t("dashboard.health.delete"),
                    true,
                    true,
                    () =>
                      run(
                        `orphan:${o.folderName}`,
                        () => api.uninstallCommunityPackage(o.folderName),
                        t("dashboard.health.deleted"),
                      ),
                  )}
                />
              ))}
            </HealthGroup>
          )}

          {compat.length > 0 && (
            <HealthGroup
              tone="violet"
              icon={<Layers className="h-4 w-4" />}
              title={t("dashboard.health.compat_title")}
              desc={t("dashboard.health.compat_desc")}
              count={compat.length}
            >
              {compat.map((m) => (
                <HealthRow
                  key={m.folderName}
                  tone="violet"
                  icon={<Plane className="h-3.5 w-3.5" />}
                  title={m.title || m.folderName}
                  subtitle={`${t("dashboard.health.built_for")} MSFS ${m.builderSim} · ${t("dashboard.health.installed_in")} MSFS ${m.communitySim}`}
                  badge={`MSFS ${m.builderSim}`}
                  action={action(
                    `compat:${m.folderName}`,
                    `${t("dashboard.health.move_to")} ${m.builderSim}`,
                    false,
                    true,
                    () =>
                      run(
                        `compat:${m.folderName}`,
                        () => api.healthMoveToSim(m.installPath, m.builderSim),
                        t("dashboard.health.moved"),
                      ),
                  )}
                />
              ))}
            </HealthGroup>
          )}

          {artifacts.length > 0 && (
            <HealthGroup
              tone="amber"
              icon={<Trash2 className="h-4 w-4" />}
              title={t("dashboard.health.artifacts_title")}
              count={artifacts.length}
            >
              {artifacts.map((a) => (
                <HealthRow
                  key={a.folderName}
                  tone="amber"
                  icon={<Package className="h-3.5 w-3.5" />}
                  title={a.title || a.folderName}
                  subtitle={
                    a.sizeBytes > 0
                      ? `${a.detail} · ${formatBytes(a.sizeBytes)}`
                      : a.detail
                  }
                  action={action(
                    `art:${a.folderName}`,
                    a.kind === "orphan_tracked"
                      ? t("dashboard.health.remove")
                      : t("dashboard.health.delete"),
                    a.kind !== "orphan_tracked",
                    a.kind !== "orphan_tracked",
                    () =>
                      run(
                        `art:${a.folderName}`,
                        a.kind === "orphan_tracked"
                          ? () => api.healthPruneTracked(a.folderName)
                          : () => api.uninstallCommunityPackage(a.folderName),
                        a.kind === "orphan_tracked"
                          ? t("dashboard.health.removed")
                          : t("dashboard.health.deleted"),
                      ),
                  )}
                />
              ))}
            </HealthGroup>
          )}
        </div>
      )}
    </Card>
  );
}

// ----------------------------------------------------------------------------
// Helpers de presentación
// ----------------------------------------------------------------------------

/** (Rediseño F2) KPI: icono en baldosa + etiqueta, valor y pista. */
function StatCard({
  icon,
  label,
  value,
  hint,
  valueTone,
  hintTone = "muted",
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  hint?: string;
  valueTone?: "warning";
  hintTone?: "muted" | "positive" | "warning";
}) {
  return (
    <div className="flex h-full items-start gap-3 rounded-xl border border-sf-border/80 bg-sf-surface p-4">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sf-raised text-brand-300">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-medium text-sf-muted">{label}</div>
        <div
          className={cn(
            "mt-0.5 text-2xl font-semibold tabular-nums",
            valueTone === "warning" ? "text-sf-warning" : "text-sf-text",
          )}
        >
          {value}
        </div>
        {hint && (
          <div
            className={cn(
              "mt-0.5 truncate text-xs",
              hintTone === "positive"
                ? "text-emerald-300"
                : hintTone === "warning"
                  ? "text-sf-warning"
                  : "text-sf-muted",
            )}
          >
            {hint}
          </div>
        )}
      </div>
    </div>
  );
}

function Card({
  title,
  icon,
  hint,
  action,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  hint?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex h-full flex-col rounded-xl border border-sf-border/80 bg-sf-surface">
      <header className="flex shrink-0 items-center justify-between gap-2 px-4 pb-1 pt-3.5">
        <div className="inline-flex min-w-0 items-center gap-2 text-sm font-semibold text-sf-text">
          {icon && <span className="shrink-0 text-sf-muted">{icon}</span>}
          <span className="truncate">{title}</span>
        </div>
        {action ??
          (hint && (
            <span className="truncate text-xs text-sf-muted">{hint}</span>
          ))}
      </header>
      {/* `flex-1` consume el espacio sobrante; el contenido se
          estira para llenar la card cuando hay otra card más alta
          al lado. */}
      <div className="flex flex-1 flex-col p-4 pt-2.5">{children}</div>
    </div>
  );
}

/** (v3.4.x) Traduce las labels que vienen del backend Rust
 *  (`stats.byType[].label`). El Rust las emite en español hardcoded
 *  ("Aeropuertos", "Aviones"…); las mapeamos al i18n del frontend
 *  para que la pantalla respete el idioma elegido. */
function translateTypeLabel(label: string): string {
  switch (label) {
    case "Aeropuertos":
      return t("addons.section.airports");
    case "Aviones":
      return t("addons.type.aircraft");
    case "Liveries":
      return t("addons.section.liveries");
    case "Instrumentos":
      return t("addons.type.instrument");
    case "Sonido / Misc":
      return t("addons.type.misc");
    case "Otros":
      return t("dashboard.type.others");
    default:
      return label;
  }
}

/** Distribución por tipo: barra apilada arriba + grid de leyenda
 *  con cuatro columnas claras (color · etiqueta · count · tamaño · %).
 *  Colores fijos para que la leyenda y la barra siempre concuerden.
 *  Las claves del map son las labels CRUDAS (en español) que vienen
 *  del backend; el render usa `translateTypeLabel()` al mostrar.
 */
function TypeBreakdown({ stats }: { stats: DashboardStats }) {
  const total = stats.byType.reduce((acc, t) => acc + t.count, 0) || 1;
  const colors: Record<string, string> = {
    Aeropuertos: "bg-emerald-500",
    Aviones: "bg-sky-500",
    Liveries: "bg-violet-500",
    Instrumentos: "bg-fuchsia-500",
    "Sonido / Misc": "bg-amber-500",
    Otros: "bg-slate-500",
  };
  const sorted = [...stats.byType].sort((a, b) => b.count - a.count);
  return (
    <div className="space-y-3">
      <div className="flex h-2.5 overflow-hidden rounded-full bg-sf-bg ring-1 ring-sf-border/70">
        {sorted.map((t) => {
          const pct = (t.count / total) * 100;
          return (
            <div
              key={t.label}
              className={`${colors[t.label] ?? "bg-slate-600"} h-full`}
              style={{ width: `${pct}%` }}
              title={`${translateTypeLabel(t.label)}: ${t.count} (${pct.toFixed(1)}%)`}
            />
          );
        })}
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {sorted.map((t) => {
          const pct = (t.count / total) * 100;
          return (
            <div
              key={t.label}
              className="flex items-center gap-2.5 rounded-lg border border-sf-border/60 bg-sf-bg/40 px-3 py-2 text-[13px]"
            >
              <span
                className={`inline-block h-2.5 w-2.5 shrink-0 rounded-sm ${colors[t.label] ?? "bg-slate-600"}`}
              />
              <span className="flex-1 truncate text-sf-text">{translateTypeLabel(t.label)}</span>
              <span className="shrink-0 font-mono text-xs text-sf-text">
                {t.count}
              </span>
              <span className="w-16 shrink-0 text-right text-[11px] tabular-nums text-sf-muted">
                {formatBytes(t.sizeBytes)}
              </span>
              <span className="w-9 shrink-0 text-right text-[11px] tabular-nums text-sf-muted/80">
                {pct.toFixed(0)}%
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Tabla de top creators con cabecera explícita ("Paquetes" / "Tamaño")
 *  y columnas alineadas. El peso queda en su propia columna numérica
 *  para no confundir con el conteo. */
function CreatorsTable({
  creators,
}: {
  creators: { creator: string; count: number; sizeBytes: number }[];
}) {
  const maxCount = creators[0]?.count ?? 1;
  return (
    <div className="overflow-hidden rounded-lg border border-sf-border/60">
      <div className="grid grid-cols-[2rem_1fr_4.5rem_5.5rem] items-center gap-2 border-b border-sf-border/60 bg-sf-raised/50 px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
        <span>#</span>
        <span>{t("dashboard.col.developer")}</span>
        <span className="text-right">{t("dashboard.col.packages")}</span>
        <span className="text-right">{t("dashboard.col.size")}</span>
      </div>
      <ul className="divide-y divide-sf-border/50">
        {creators.map((c, idx) => {
          const pct = (c.count / maxCount) * 100;
          return (
            <li
              key={c.creator}
              className="grid grid-cols-[2rem_1fr_4.5rem_5.5rem] items-center gap-2 px-3 py-2 text-[13px]"
            >
              <span className="font-mono text-[11px] text-sf-muted">
                #{idx + 1}
              </span>
              <div className="min-w-0">
                <div className="truncate text-sf-text">{c.creator}</div>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-sf-raised">
                  <div
                    className="h-full rounded-full bg-brand-500/70"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
              <span className="text-right font-mono tabular-nums text-sf-text">
                {c.count}
              </span>
              <span className="text-right font-mono text-xs tabular-nums text-sf-muted">
                {formatBytes(c.sizeBytes)}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** Tabla de top 10 más grandes — mismo patrón columnar. La barra
 *  representa proporción contra el más grande (no contra total),
 *  para que sea intuitivo "este pesa la mitad del top". */
function LargestTable({
  rows,
  maxBytes,
}: {
  rows: {
    folderName: string;
    title: string;
    creator: string | null;
    sizeBytes: number | null;
  }[];
  maxBytes: number;
}) {
  return (
    <div className="overflow-hidden rounded-lg border border-sf-border/60">
      <div className="grid grid-cols-[2rem_1fr_5.5rem] items-center gap-2 border-b border-sf-border/60 bg-sf-raised/50 px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
        <span>#</span>
        <span>{t("dashboard.col.package")}</span>
        <span className="text-right">{t("dashboard.col.size")}</span>
      </div>
      <ul className="divide-y divide-sf-border/50">
        {rows.map((p, idx) => {
          const bytes = p.sizeBytes ?? 0;
          const pct = (bytes / maxBytes) * 100;
          return (
            <li
              key={p.folderName}
              className="grid grid-cols-[2rem_1fr_5.5rem] items-center gap-2 px-3 py-2 text-[13px]"
            >
              <span className="font-mono text-[11px] text-sf-muted">
                #{idx + 1}
              </span>
              <div className="min-w-0">
                <div className="truncate text-sf-text">{p.title}</div>
                <div className="truncate text-[11px] text-sf-muted">
                  {p.creator ?? t("dashboard.unknown_author")}
                </div>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-sf-raised">
                  <div
                    className="h-full rounded-full bg-sky-500/70"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
              <span className="text-right font-mono text-xs tabular-nums text-sf-text">
                {formatBytes(bytes)}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function formatBytes(n: number): string {
  if (!isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}
