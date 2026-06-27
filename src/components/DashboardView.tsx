import { useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  Download,
  HardDrive,
  Landmark,
  Loader2,
  Package,
  Plane,
  Rocket,
  Sparkles,
  Trophy,
  Truck,
  Users,
} from "lucide-react";
import type { DashboardStats } from "../lib/types";
import { api } from "../lib/tauri";
import { useCommunityStore } from "../stores/useCommunityStore";
import {
  useGsxUpdateStore,
  gsxUpdateVisible,
} from "../stores/useGsxUpdateStore";
import { derivedType } from "../lib/packageType";
import { t } from "../lib/i18n";

/**
 * Dashboard — totales agregados de Community.
 *
 * Layout responsive: en pantallas pequeñas todo apila en columna
 * única; en md/lg/xl crece a 4 KPIs arriba y dos columnas debajo.
 * El bloque "más grandes" tiene un gráfico de barras horizontal
 * con el peso real, no porcentaje del top 1 (eso da mejor
 * intuición del consumo real de disco).
 */
export function DashboardView() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // (v6.2.4) Aviso de update de GSX (FSDreamTeam) — el chequeo horario vive en
  // App.tsx; aquí leemos el resultado compartido.
  const gsxInfo = useGsxUpdateStore((s) => s.info);
  const gsxRequestInstall = useGsxUpdateStore((s) => s.requestInstall);
  const showGsx = gsxUpdateVisible(gsxInfo);

  // Recalculamos automáticamente cuando termina un scan de Community
  // — el usuario no necesita pulsar "refrescar". `scanning` flipa
  // false → true → false en cada scan; observamos la transición a
  // false para recargar las stats.
  const scanning = useCommunityStore((s) => s.scanning);
  const packages = useCommunityStore((s) => s.packages);
  const packagesCount = packages.length;

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

  const load = () => {
    setLoading(true);
    setError(null);
    api
      .getDashboardStats()
      .then((s) => setStats(s))
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

  return (
    <div className="space-y-4">
      <header>
        <h2 className="text-base font-semibold text-slate-100">
          {t("dashboard.title.community")}
        </h2>
      </header>

      {/* (v6.2.4) Banner de update de GSX — clic abre el FSDT Installer. */}
      {showGsx && gsxInfo && (
        <button
          onClick={() => gsxRequestInstall()}
          className="flex w-full items-center gap-3 rounded-xl border border-cyan-500/30 bg-cyan-500/10 px-4 py-3 text-left transition-colors hover:bg-cyan-500/15"
        >
          <Truck className="h-5 w-5 shrink-0 text-cyan-300" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold text-cyan-100">
              {t("notifications.gsx_update")}
            </div>
            <div className="text-xs text-cyan-200/80">
              {gsxInfo.installedVersion} →{" "}
              <strong>{gsxInfo.latestVersion}</strong>
              {gsxInfo.latestDate ? ` · ${gsxInfo.latestDate}` : ""}
            </div>
          </div>
          <span className="inline-flex shrink-0 items-center gap-1 rounded-md bg-cyan-500/30 px-2.5 py-1 text-[11px] font-medium text-cyan-100">
            <Download className="h-3 w-3" /> {t("notifications.gsx_update_now")}
          </span>
        </button>
      )}

      {error && (
        <div className="flex items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
          <AlertCircle className="h-3.5 w-3.5" />
          <span>{error}</span>
        </div>
      )}

      {!stats && loading && (
        <div className="flex items-center justify-center rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-12 text-sm text-slate-500">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("dashboard.computing")}
        </div>
      )}

      {stats && stats.totalPackages === 0 && !loading && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-12 text-center text-sm text-slate-500">
          <Package className="mx-auto mb-3 h-8 w-8 text-slate-700" />
          <p>{t("dashboard.empty.title")}</p>
          <p className="mt-1 text-xs text-slate-600">
            {t("dashboard.empty.hint")}
          </p>
        </div>
      )}

      {stats && stats.totalPackages > 0 && (
        <>
          {/* KPIs — 4 columnas en md+, 2 en sm, 1 en xs. */}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <KpiCard
              icon={<Package className="h-4 w-4" />}
              label={t("dashboard.kpi.total_packages")}
              value={stats.totalPackages.toLocaleString("es-ES")}
              hint={t("dashboard.kpi.total_packages.hint")}
              tone="brand"
            />
            <KpiCard
              icon={<HardDrive className="h-4 w-4" />}
              label={t("dashboard.kpi.disk_space")}
              value={formatBytes(stats.totalSizeBytes)}
              hint={t("dashboard.kpi.disk_space.hint")}
              tone="cyan"
            />
            <KpiCard
              icon={<Landmark className="h-4 w-4" />}
              label={t("dashboard.kpi.airports")}
              value={stats.airportsCount.toLocaleString("es-ES")}
              hint={t("dashboard.kpi.airports.hint", { aircraft: aircraftCount, liveries: liveriesCount })}
              tone="emerald"
            />
            <KpiCard
              icon={<Sparkles className="h-4 w-4" />}
              label={t("dashboard.kpi.updates")}
              value={stats.updatesAvailable.toLocaleString("es-ES")}
              hint={
                stats.updatesAvailable === 0
                  ? t("dashboard.kpi.updates.all_clear")
                  : t("dashboard.kpi.updates.check_notifications")
              }
              tone={stats.updatesAvailable > 0 ? "amber" : "slate"}
            />
          </div>

          <Card title={t("dashboard.card.by_type")} icon={<Rocket className="h-3.5 w-3.5" />}>
            <TypeBreakdown stats={stats} />
          </Card>

          {/* Top devs y largest packages se estiran a la misma
              altura usando `grid-flow-row-dense` + `items-stretch`,
              y dentro las cards crecen con `h-full + flex-col`. */}
          <div className="grid items-stretch gap-3 lg:grid-cols-2">
            <Card
              title={t("dashboard.card.top_devs")}
              icon={<Users className="h-3.5 w-3.5" />}
              hint={t("dashboard.card.top_devs.hint")}
            >
              {stats.topCreators.length === 0 ? (
                <p className="px-1 py-2 text-xs text-slate-500">
                  {t("dashboard.card.top_devs.empty")}
                </p>
              ) : (
                <CreatorsTable creators={stats.topCreators} />
              )}
            </Card>

            <Card
              title={t("dashboard.card.largest")}
              icon={<Trophy className="h-3.5 w-3.5" />}
              hint={t("dashboard.card.largest.hint")}
            >
              {stats.largestPackages.length === 0 ? (
                <p className="px-1 py-2 text-xs text-slate-500">
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
            <Card title={t("dashboard.card.recently_added")} icon={<Plane className="h-3.5 w-3.5" />}>
              <ul className="grid grid-cols-1 gap-1.5 sm:grid-cols-2 xl:grid-cols-3">
                {stats.recentlyAdded.map((p) => (
                  <li
                    key={p.folderName}
                    className="flex items-center gap-2 rounded-md border border-slate-800/60 bg-slate-950/40 px-2.5 py-1.5 text-xs"
                  >
                    <Package className="h-3.5 w-3.5 shrink-0 text-slate-500" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-slate-200">{p.title}</div>
                      <div className="truncate text-[10px] text-slate-500">
                        {p.creator ?? t("dashboard.unknown_author")}
                        {p.sizeBytes != null && ` · ${formatBytes(p.sizeBytes)}`}
                      </div>
                    </div>
                  </li>
                ))}
              </ul>
            </Card>
          )}
        </>
      )}
    </div>
  );
}

// ----------------------------------------------------------------------------
// Helpers de presentación
// ----------------------------------------------------------------------------

type Tone = "brand" | "cyan" | "emerald" | "amber" | "slate";

function KpiCard({
  icon,
  label,
  value,
  hint,
  tone,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  hint?: string;
  tone: Tone;
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
      <div className={`inline-flex items-center gap-1.5 ${toneClass(tone)}`}>
        {icon}
        <span className="text-[11px] font-medium uppercase tracking-wider">
          {label}
        </span>
      </div>
      <div className="mt-1 text-2xl font-semibold text-slate-100">{value}</div>
      {hint && <div className="mt-0.5 text-[11px] text-slate-500">{hint}</div>}
    </div>
  );
}

function toneClass(t: Tone): string {
  switch (t) {
    case "brand":
      return "text-brand-300";
    case "cyan":
      return "text-cyan-300";
    case "emerald":
      return "text-emerald-300";
    case "amber":
      return "text-amber-300";
    default:
      return "text-slate-400";
  }
}

function Card({
  title,
  icon,
  hint,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex h-full flex-col rounded-xl border border-slate-800 bg-slate-900/40">
      <header className="flex shrink-0 items-center justify-between gap-2 border-b border-slate-800 px-4 py-2.5">
        <div className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-300">
          {icon}
          {title}
        </div>
        {hint && (
          <span className="text-[10px] font-normal normal-case text-slate-500">
            {hint}
          </span>
        )}
      </header>
      {/* `flex-1` consume el espacio sobrante; el contenido se
          estira para llenar la card cuando hay otra card más alta
          al lado. */}
      <div className="flex flex-1 flex-col p-3">{children}</div>
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
      <div className="flex h-2.5 overflow-hidden rounded-full bg-slate-950 ring-1 ring-slate-800">
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
      <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2 lg:grid-cols-3">
        {sorted.map((t) => {
          const pct = (t.count / total) * 100;
          return (
            <div
              key={t.label}
              className="flex items-center gap-2 rounded-md border border-slate-800/60 bg-slate-950/40 px-2.5 py-1.5 text-xs"
            >
              <span
                className={`inline-block h-2.5 w-2.5 shrink-0 rounded-sm ${colors[t.label] ?? "bg-slate-600"}`}
              />
              <span className="flex-1 truncate text-slate-200">{translateTypeLabel(t.label)}</span>
              <span className="shrink-0 font-mono text-[11px] text-slate-300">
                {t.count}
              </span>
              <span className="w-16 shrink-0 text-right text-[10px] tabular-nums text-slate-500">
                {formatBytes(t.sizeBytes)}
              </span>
              <span className="w-9 shrink-0 text-right text-[10px] tabular-nums text-slate-600">
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
 *  y columnas alineadas. Reemplaza la barra de progreso por el rank;
 *  el peso queda en su propia columna numérica para no confundir
 *  con el conteo. */
function CreatorsTable({
  creators,
}: {
  creators: { creator: string; count: number; sizeBytes: number }[];
}) {
  const maxCount = creators[0]?.count ?? 1;
  return (
    <div className="overflow-hidden rounded-md border border-slate-800/60">
      <div className="grid grid-cols-[2rem_1fr_4rem_5.5rem] items-center gap-2 border-b border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-slate-500">
        <span>#</span>
        <span>{t("dashboard.col.developer")}</span>
        <span className="text-right">{t("dashboard.col.packages")}</span>
        <span className="text-right">{t("dashboard.col.size")}</span>
      </div>
      <ul className="divide-y divide-slate-800/60">
        {creators.map((c, idx) => {
          const pct = (c.count / maxCount) * 100;
          return (
            <li
              key={c.creator}
              className="grid grid-cols-[2rem_1fr_4rem_5.5rem] items-center gap-2 px-2.5 py-1.5 text-xs"
            >
              <span className="font-mono text-[10px] text-slate-500">
                #{idx + 1}
              </span>
              <div className="min-w-0">
                <div className="truncate text-slate-200">{c.creator}</div>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-slate-900">
                  <div
                    className="h-full rounded-full bg-brand-500/70"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
              <span className="text-right font-mono tabular-nums text-slate-200">
                {c.count}
              </span>
              <span className="text-right font-mono text-[11px] tabular-nums text-slate-400">
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
    <div className="overflow-hidden rounded-md border border-slate-800/60">
      <div className="grid grid-cols-[2rem_1fr_5.5rem] items-center gap-2 border-b border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-slate-500">
        <span>#</span>
        <span>{t("dashboard.col.package")}</span>
        <span className="text-right">{t("dashboard.col.size")}</span>
      </div>
      <ul className="divide-y divide-slate-800/60">
        {rows.map((p, idx) => {
          const bytes = p.sizeBytes ?? 0;
          const pct = (bytes / maxBytes) * 100;
          return (
            <li
              key={p.folderName}
              className="grid grid-cols-[2rem_1fr_5.5rem] items-center gap-2 px-2.5 py-1.5 text-xs"
            >
              <span className="font-mono text-[10px] text-slate-500">
                #{idx + 1}
              </span>
              <div className="min-w-0">
                <div className="truncate text-slate-200">{p.title}</div>
                <div className="truncate text-[10px] text-slate-500">
                  {p.creator ?? t("dashboard.unknown_author")}
                </div>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-slate-900">
                  <div
                    className="h-full rounded-full bg-cyan-500/70"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
              <span className="text-right font-mono text-[11px] tabular-nums text-slate-300">
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
