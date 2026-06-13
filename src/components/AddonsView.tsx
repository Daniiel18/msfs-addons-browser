import { useEffect, useMemo, useState } from "react";
import {
  Boxes,
  CircleOff,
  Cog,
  GitBranch,
  HardDrive,
  LayoutGrid,
  Loader2,
  Music,
  Package,
  Pencil,
  Plane,
  Palette,
  Power,
  PowerOff,
  RefreshCcw,
  Search,
  HelpCircle,
} from "lucide-react";
import type { CommunityPackage, PmdgLivery } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useToastStore } from "../stores/useToastStore";
import { PackageDetailModal } from "./PackageDetailModal";
import { LinkMapView } from "./LinkMapView";
import {
  derivedType,
  isAddon,
  looksLikePlaceholderTitle,
  type DerivedType,
} from "../lib/packageType";
import { useThumbnail } from "../lib/thumbnails";
import { disambiguateTitles } from "../lib/displayTitle";
import { ToggleSwitch, usePackageToggle } from "./AddonToggle";
import { AddonFallbackArt } from "./AddonArt";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/** (v4.25.0) Píldoras de categoría estilo "My Content": All /
 *  Aircraft / Liveries / Utilities / Others. SIN Airports — los
 *  aeropuertos se centralizan en la pestaña Map (scenery). */
type PillFilter = "ALL" | "AIRCRAFT" | "LIVERY" | "UTILITIES" | "OTHERS";

function pillMatches(pill: PillFilter, ty: DerivedType): boolean {
  switch (pill) {
    case "ALL":
      return true;
    case "AIRCRAFT":
      return ty === "AIRCRAFT";
    case "LIVERY":
      return ty === "LIVERY";
    case "UTILITIES":
      return ty === "INSTRUMENT";
    case "OTHERS":
      return ty === "MISC" || ty === "UNKNOWN";
  }
}

/**
 * Vista de addons no-escenario: aviones, liveries, instrumentos,
 * misc, unknown.
 *
 * (v4.25.0) Rediseño "panel de control" estilo My Content:
 *   · Fila de métricas: addons totales, activos, desactivados y
 *     storage total en disco.
 *   · Batch actions: Enable All / Disable All (con confirmación) +
 *     Refresh (re-scan de Community).
 *   · Píldoras de categoría (sin Airports — viven en el Map).
 *   · Cards con toggle físico para encender/apagar el addon en el
 *     sim (renombra layout.json; cascada vía Link Map) + botón de
 *     metadatos que abre el modal de detalle.
 *   · Vista alternativa **Link Map**: grafo de dependencias con
 *     activación en cascada (LinkMapView).
 */
export function AddonsView() {
  const allPackages = useCommunityStore((s) => s.packages);
  const detailsFor = useCommunityStore((s) => s.detailsFor);
  const openDetails = useCommunityStore((s) => s.openDetails);
  const updates = useCommunityStore((s) => s.updates);
  const scanning = useCommunityStore((s) => s.scanning);
  const rescan = useCommunityStore((s) => s.rescan);
  const setManyEnabled = useCommunityStore((s) => s.setManyEnabled);
  const pushToast = useToastStore((s) => s.push);

  const [filter, setFilter] = useState("");
  const [typeFilter, setTypeFilter] = useState<PillFilter>("ALL");
  // (v4.25.0) Vista activa: grid clásico o grafo de dependencias.
  const [view, setView] = useState<"grid" | "linkmap">("grid");
  // (v4.25.0) Confirmación pendiente de un batch (null = ninguna).
  const [confirmBatch, setConfirmBatch] = useState<"enable" | "disable" | null>(
    null,
  );
  const [batchBusy, setBatchBusy] = useState(false);

  // Excluimos escenarios — son del MapView. Lo que queda son
  // AIRCRAFT, LIVERY (derivado), INSTRUMENT, MISC, UNKNOWN.
  // (v4.26.0) Los títulos duplicados (manifest genérico: dos
  // "737-800", varias "Liveries") se desambiguan con un sufijo
  // derivado del folder — el mismo displayTitle se usa en el grid y
  // en el Link Map.
  const addons = useMemo(() => {
    const base = allPackages.filter(isAddon);
    const display = disambiguateTitles(base);
    return base.map((p) => ({
      p: display.has(p.folderName)
        ? { ...p, title: display.get(p.folderName)! }
        : p,
      t: derivedType(p),
    }));
  }, [allPackages]);

  const counts = useMemo(() => {
    const m = new Map<DerivedType, number>();
    for (const { t } of addons) m.set(t, (m.get(t) ?? 0) + 1);
    return m;
  }, [addons]);

  // Tipos presentes en el orden canónico.
  const presentTypes = useMemo<DerivedType[]>(() => {
    const order: DerivedType[] = [
      "AIRCRAFT",
      "LIVERY",
      "INSTRUMENT",
      "MISC",
      "UNKNOWN",
    ];
    return order.filter((t) => (counts.get(t) ?? 0) > 0);
  }, [counts]);

  // (v4.25.0) Métricas globales del panel. Storage = suma de bytes de
  // todos los addons no-escenario; enabled trata undefined (demo)
  // como activo.
  const metrics = useMemo(() => {
    let enabled = 0;
    let bytes = 0;
    for (const { p } of addons) {
      if (p.enabled !== false) enabled += 1;
      bytes += p.sizeBytes ?? 0;
    }
    return {
      total: addons.length,
      enabled,
      disabled: addons.length - enabled,
      bytes,
    };
  }, [addons]);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return addons.filter(({ p, t }) => {
      if (!pillMatches(typeFilter, t)) return false;
      if (!q) return true;
      return [p.title, p.creator, p.folderName]
        .filter(Boolean)
        .some((s) => s!.toLowerCase().includes(q));
    });
  }, [addons, filter, typeFilter]);

  // (v4.25.0) Batch Enable/Disable sobre los addons VISIBLES (lo que
  // el filtro actual muestra es lo que se toca — predecible).
  const runBatch = async (enabled: boolean) => {
    setBatchBusy(true);
    try {
      const folders = visible.map(({ p }) => p.folderName);
      const report = await setManyEnabled(folders, enabled);
      pushToast({
        kind: report.failed.length > 0 ? "error" : "success",
        title: enabled
          ? t("addons.batch.enabled_toast", { n: report.changed.length })
          : t("addons.batch.disabled_toast", { n: report.changed.length }),
        message:
          report.failed.length > 0
            ? t("addons.batch.failed_toast", { n: report.failed.length })
            : undefined,
      });
    } catch (e) {
      pushToast({ kind: "error", title: t("addons.toggle.error"), message: String(e) });
    } finally {
      setBatchBusy(false);
      setConfirmBatch(null);
    }
  };

  // Agrupar por tipo cuando no hay filtro de tipo activo. En cada
  // grupo respetamos el orden alfabético del título.
  const grouped = useMemo(() => {
    const m = new Map<DerivedType, { p: CommunityPackage; t: DerivedType }[]>();
    for (const item of visible) {
      const arr = m.get(item.t) ?? [];
      arr.push(item);
      m.set(item.t, arr);
    }
    for (const [, arr] of m) {
      arr.sort((a, b) => a.p.title.localeCompare(b.p.title));
    }
    return m;
  }, [visible]);

  const detailsPkg = useMemo(
    () => allPackages.find((p) => p.folderName === detailsFor) ?? null,
    [allPackages, detailsFor],
  );
  const detailsUpdate = useMemo(
    () =>
      detailsFor
        ? updates.find((u) => u.folderName === detailsFor) ?? null
        : null,
    [updates, detailsFor],
  );

  return (
    <div className="space-y-3">
      {/* Header — título + métricas globales + batch actions + search. */}
      <header className="space-y-3 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-3">
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2">
            <Boxes className="h-4 w-4 text-brand-300" />
            <h2 className="text-sm font-semibold text-slate-100">
              {t("addons.title")}
            </h2>
          </div>

          {/* (v4.25.0) Fila de métricas con micro-iconos. */}
          <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
            <MetricChip
              icon={<Package className="h-3 w-3 text-brand-300" />}
              value={metrics.total}
              label={t("addons.metrics.addons")}
            />
            <MetricChip
              icon={<Power className="h-3 w-3 text-emerald-400" />}
              value={metrics.enabled}
              label={t("addons.metrics.enabled")}
            />
            <MetricChip
              icon={<CircleOff className="h-3 w-3 text-slate-500" />}
              value={metrics.disabled}
              label={t("addons.metrics.disabled")}
            />
            <MetricChip
              icon={<HardDrive className="h-3 w-3 text-sky-300" />}
              value={formatStorage(metrics.bytes)}
              label={t("addons.metrics.storage")}
            />
          </div>

          <div className="ml-auto flex flex-wrap items-center gap-2">
            {/* (v4.25.0) Batch actions. Operan sobre los addons
                visibles con el filtro actual. */}
            <button
              onClick={() => setConfirmBatch("enable")}
              disabled={batchBusy || visible.length === 0}
              className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1.5 text-[11px] font-medium text-emerald-200 hover:border-emerald-400 hover:bg-emerald-500/20 disabled:opacity-50"
            >
              <Power className="h-3 w-3" />
              {t("addons.batch.enable_all")}
            </button>
            <button
              onClick={() => setConfirmBatch("disable")}
              disabled={batchBusy || visible.length === 0}
              className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-800/60 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 hover:border-rose-400/40 hover:bg-rose-500/10 hover:text-rose-200 disabled:opacity-50"
            >
              <PowerOff className="h-3 w-3" />
              {t("addons.batch.disable_all")}
            </button>
            <button
              onClick={() => void rescan()}
              disabled={scanning}
              className="inline-flex items-center gap-1.5 rounded-lg border border-sky-500/40 bg-sky-500/10 px-2.5 py-1.5 text-[11px] font-medium text-sky-200 hover:border-sky-400 hover:bg-sky-500/20 disabled:opacity-50"
            >
              {scanning ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <RefreshCcw className="h-3 w-3" />
              )}
              {t("addons.batch.refresh")}
            </button>

            {/* (v4.25.0) Switch de vista Grid ⇄ Link Map. */}
            <div className="flex overflow-hidden rounded-lg border border-slate-700">
              <button
                onClick={() => setView("grid")}
                title={t("addons.view.grid")}
                className={`inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] font-medium transition-colors ${
                  view === "grid"
                    ? "bg-brand-500/20 text-brand-100"
                    : "bg-slate-900/40 text-slate-400 hover:text-slate-200"
                }`}
              >
                <LayoutGrid className="h-3 w-3" />
                {t("addons.view.grid")}
              </button>
              <button
                onClick={() => setView("linkmap")}
                title={t("addons.view.linkmap")}
                className={`inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] font-medium transition-colors ${
                  view === "linkmap"
                    ? "bg-amber-500/20 text-amber-100"
                    : "bg-slate-900/40 text-slate-400 hover:text-slate-200"
                }`}
              >
                <GitBranch className="h-3 w-3" />
                {t("addons.view.linkmap")}
              </button>
            </div>

            <div className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
              <input
                type="text"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={t("addons.search.placeholder")}
                className="w-48 rounded-md border border-slate-800 bg-slate-950/50 py-1.5 pl-8 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30 xl:w-64"
              />
            </div>
          </div>
        </div>
      </header>

      {/* (v4.25.0) Píldoras de categoría — All / Aircraft / Liveries /
          Utilities / Others. SIN Airports (viven en Map scenery). */}
      <div className="flex flex-wrap gap-1.5">
        <TypeChip
          active={typeFilter === "ALL"}
          onClick={() => setTypeFilter("ALL")}
          icon={<Package className="h-3.5 w-3.5" />}
          label={t("addons.chip.all")}
          count={addons.length}
        />
        <TypeChip
          active={typeFilter === "AIRCRAFT"}
          onClick={() => setTypeFilter("AIRCRAFT")}
          icon={<Plane className="h-3.5 w-3.5" />}
          label={t("addons.chip.aircraft")}
          count={counts.get("AIRCRAFT") ?? 0}
        />
        <TypeChip
          active={typeFilter === "LIVERY"}
          onClick={() => setTypeFilter("LIVERY")}
          icon={<Palette className="h-3.5 w-3.5" />}
          label={t("addons.chip.liveries")}
          count={counts.get("LIVERY") ?? 0}
        />
        <TypeChip
          active={typeFilter === "UTILITIES"}
          onClick={() => setTypeFilter("UTILITIES")}
          icon={<Cog className="h-3.5 w-3.5" />}
          label={t("addons.chip.utilities")}
          count={counts.get("INSTRUMENT") ?? 0}
        />
        <TypeChip
          active={typeFilter === "OTHERS"}
          onClick={() => setTypeFilter("OTHERS")}
          icon={<Music className="h-3.5 w-3.5" />}
          label={t("addons.chip.others")}
          count={(counts.get("MISC") ?? 0) + (counts.get("UNKNOWN") ?? 0)}
        />
      </div>

      {/* Cuerpo: grafo o grid según vista activa. */}
      {view === "linkmap" ? (
        <LinkMapView addons={addons} />
      ) : (
        <>
          {visible.length === 0 ? (
            <EmptyState
              hasAny={addons.length > 0}
              hasFilter={!!filter || typeFilter !== "ALL"}
            />
          ) : typeFilter !== "ALL" || filter.trim() !== "" ? (
            // Lista plana — el usuario ya filtró, no agrupamos.
            <CardsGrid items={visible} updates={updates} onOpen={openDetails} />
          ) : (
            // Agrupado por tipo con cabeceras.
            <div className="space-y-5">
              {presentTypes.map((t) => {
                const items = grouped.get(t) ?? [];
                if (items.length === 0) return null;
                return (
                  <section key={t}>
                    <h3 className="mb-2 inline-flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                      {typeIcon(t)}
                      {typeLabel(t)}
                      <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-500 normal-case tracking-normal">
                        {items.length}
                      </span>
                    </h3>
                    <CardsGrid
                      items={items}
                      updates={updates}
                      onOpen={openDetails}
                    />
                  </section>
                );
              })}
            </div>
          )}

          {/* (v3.0.0 / v3.1.0 / v3.1.3) Sección Aircraft Liveries —
              escaneo dedicado de paquetes `*-aircraft-*` (PMDG + Fenix +
              iniBuilds + FlyByWire…) con parsing del aircraft.cfg.
              Recibe el filtro de búsqueda global del header.
              v3.1.3: SÓLO se muestra cuando typeFilter es "ALL" o
              "LIVERY". Antes aparecía también con AIRCRAFT / MISC /
              UNKNOWN seleccionados, lo que el usuario reportó como
              "se cuela en todos los filtros". */}
          {(typeFilter === "ALL" || typeFilter === "LIVERY") && (
            <AircraftLiveriesSection filter={filter} />
          )}
        </>
      )}

      {detailsPkg && (
        <PackageDetailModal
          pkg={detailsPkg}
          update={detailsUpdate}
          onClose={() => openDetails(null)}
        />
      )}

      {/* (v4.25.0) Confirmación de batch — apagar/encender N addons de
          golpe toca el filesystem de Community; merece un "¿seguro?". */}
      {confirmBatch && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 backdrop-blur-sm"
          onClick={() => !batchBusy && setConfirmBatch(null)}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className="w-[min(420px,calc(100vw-2rem))] rounded-xl border border-slate-800 bg-slate-950 p-5 shadow-2xl ring-1 ring-slate-800"
          >
            <h3 className="text-sm font-semibold text-slate-100">
              {confirmBatch === "enable"
                ? t("addons.batch.confirm_enable.title", { n: visible.length })
                : t("addons.batch.confirm_disable.title", { n: visible.length })}
            </h3>
            <p className="mt-2 text-xs text-slate-400">
              {confirmBatch === "enable"
                ? t("addons.batch.confirm_enable.body")
                : t("addons.batch.confirm_disable.body")}
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                onClick={() => setConfirmBatch(null)}
                disabled={batchBusy}
                className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-200 hover:bg-slate-800 disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void runBatch(confirmBatch === "enable")}
                disabled={batchBusy}
                className={`inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium disabled:opacity-50 ${
                  confirmBatch === "enable"
                    ? "bg-emerald-500 text-emerald-950 hover:bg-emerald-400"
                    : "bg-rose-500 text-rose-950 hover:bg-rose-400"
                }`}
              >
                {batchBusy ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : confirmBatch === "enable" ? (
                  <Power className="h-3 w-3" />
                ) : (
                  <PowerOff className="h-3 w-3" />
                )}
                {t("common.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** (v4.25.0) Chip de métrica del header: icono + valor + label. */
function MetricChip({
  icon,
  value,
  label,
}: {
  icon: React.ReactNode;
  value: number | string;
  label: string;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-950/50 px-2 py-1">
      {icon}
      <span className="font-semibold tabular-nums text-slate-100">{value}</span>
      <span className="text-slate-500">{label}</span>
    </span>
  );
}

/** (v4.25.0) Formatea bytes como storage humano (GB con 2 decimales
 *  estilo "205.86 GB", MB para totales chicos). */
export function formatStorage(bytes: number): string {
  const gb = bytes / 1_000_000_000;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(bytes / 1_000_000).toFixed(0)} MB`;
}

// ----------------------------------------------------------------------------
// Aircraft Liveries section (v3.0.0 / v3.1.0)
// ----------------------------------------------------------------------------

/** Mapa de vendor → label amigable + accent color. */
function vendorLabel(vendor: string): { name: string; tone: string } {
  switch (vendor) {
    case "pmdg":
      return { name: "PMDG", tone: "text-amber-300" };
    case "fnx":
      return { name: "Fenix", tone: "text-sky-300" };
    case "inibuilds":
      return { name: "iniBuilds", tone: "text-emerald-300" };
    case "fbw":
      return { name: "FlyByWire", tone: "text-violet-300" };
    case "aerosoft":
      return { name: "Aerosoft", tone: "text-rose-300" };
    case "headwind":
      return { name: "Headwind", tone: "text-orange-300" };
    case "leonardo":
      return { name: "Leonardo", tone: "text-purple-300" };
    case "dc-designs":
      return { name: "DC Designs", tone: "text-cyan-300" };
    case "justflight":
      return { name: "JustFlight", tone: "text-lime-300" };
    default:
      return {
        name: vendor.charAt(0).toUpperCase() + vendor.slice(1),
        tone: "text-slate-300",
      };
  }
}

function modelLabel(vendor: string, m: string): string {
  // PMDG conocidos.
  const pmdgMap: Record<string, string> = {
    "738": "737-800",
    "739": "737-900",
    "77er": "777-200ER",
    "77w": "777-300ER",
    "77f": "777F",
    "744": "747-400",
    "748": "747-8",
  };
  if (vendor === "pmdg" && pmdgMap[m]) return pmdgMap[m];
  if (vendor === "fnx" && m === "320") return "A320";
  if (vendor === "inibuilds" && m === "a350") return "A350";
  if (vendor === "fbw" && m === "a32nx") return "A32NX";
  return m.toUpperCase();
}

function AircraftLiveriesSection({ filter }: { filter: string }) {
  const [liveries, setLiveries] = useState<PmdgLivery[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // (v3.2.0) Estado expandido POR VENDOR — cada uno con su propio
  // toggle. Default todos colapsados al cargar para no inundar la UI.
  const [expandedVendors, setExpandedVendors] = useState<Set<string>>(
    new Set(),
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .listPmdgLiveries()
      .then((data) => {
        if (cancelled) return;
        setLiveries(data);
        // Auto-expandir el primer vendor con liveries para que el
        // usuario vea contenido al primer paint.
        const firstVendor = data[0]?.vendor;
        if (firstVendor) {
          setExpandedVendors(new Set([firstVendor]));
        }
        setLoading(false);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Aplica el filtro global del header sobre los liveries.
  const q = filter.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!liveries) return [] as PmdgLivery[];
    if (!q) return liveries;
    return liveries.filter((l) =>
      [
        l.title,
        l.airline,
        l.tailNumber,
        l.variation,
        l.texture,
        l.packageFolder,
        l.vendor,
        l.model,
        `${vendorLabel(l.vendor).name} ${modelLabel(l.vendor, l.model)}`,
      ]
        .filter(Boolean)
        .some((s) => s!.toLowerCase().includes(q)),
    );
  }, [liveries, q]);

  if (loading) {
    return (
      <section className="rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-8 text-center text-xs text-slate-500">
        Escaneando liveries de aircraft addons…
      </section>
    );
  }
  if (error) {
    return (
      <section className="rounded-xl border border-rose-500/30 bg-rose-500/10 px-4 py-3 text-xs text-rose-200">
        Error escaneando liveries: {error}
      </section>
    );
  }
  if (!liveries || liveries.length === 0) {
    return null;
  }
  if (q && filtered.length === 0) {
    return null;
  }

  // (v3.2.0) Group by VENDOR — cada vendor su propio collapse. Antes
  // todo en un único collapse "Aircraft Liveries". Ahora un collapse
  // independiente por vendor (PMDG Liveries / Fenix Liveries / etc.)
  // con los modelos del vendor adentro.
  const byVendor = new Map<string, PmdgLivery[]>();
  for (const l of filtered) {
    const arr = byVendor.get(l.vendor) ?? [];
    arr.push(l);
    byVendor.set(l.vendor, arr);
  }
  const vendorEntries = Array.from(byVendor.entries()).sort(([a], [b]) =>
    vendorLabel(a).name.localeCompare(vendorLabel(b).name),
  );

  const toggleVendor = (vendor: string) => {
    setExpandedVendors((prev) => {
      const next = new Set(prev);
      if (next.has(vendor)) next.delete(vendor);
      else next.add(vendor);
      return next;
    });
  };

  return (
    <div className="space-y-3">
      {vendorEntries.map(([vendor, items]) => {
        const v = vendorLabel(vendor);
        const isOpen = expandedVendors.has(vendor);
        // Subgrupos por modelo dentro de cada vendor.
        const byModel = new Map<string, PmdgLivery[]>();
        for (const l of items) {
          const arr = byModel.get(l.model) ?? [];
          arr.push(l);
          byModel.set(l.model, arr);
        }
        return (
          <section
            key={vendor}
            className="rounded-xl border border-slate-800 bg-slate-900/40"
          >
            <header
              className="flex cursor-pointer items-center gap-2 border-b border-slate-800 px-4 py-3 hover:bg-slate-900/60"
              onClick={() => toggleVendor(vendor)}
            >
              <Plane className={`h-4 w-4 ${v.tone}`} />
              <h3 className={`text-sm font-semibold ${v.tone}`}>
                {v.name} Liveries
              </h3>
              <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-400">
                {items.length}
              </span>
              <span className="ml-auto text-[11px] text-slate-500">
                {isOpen ? "↓ Colapsar" : "→ Expandir"}
              </span>
            </header>
            {isOpen && (
              <div className="space-y-5 p-4">
                {Array.from(byModel.entries())
                  .sort(([a], [b]) => a.localeCompare(b))
                  .map(([model, modelItems]) => (
                    <div key={model}>
                      <h4 className="mb-2 inline-flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                        {modelLabel(vendor, model)}
                        <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-500 normal-case tracking-normal">
                          {modelItems.length}
                        </span>
                      </h4>
                      <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                        {modelItems.map((liv, idx) => (
                          <LiveryCard
                            key={`${liv.title}-${idx}`}
                            liv={liv}
                          />
                        ))}
                      </ul>
                    </div>
                  ))}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

function LiveryCard({ liv }: { liv: PmdgLivery }) {
  return (
    <li className="group flex h-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950/60 transition-colors hover:border-brand-500/40 hover:bg-slate-900">
      {/* Thumbnail rectangular arriba (~16:9), igual que PackageCard. */}
      <div className="relative h-28 w-full overflow-hidden bg-slate-900">
        {liv.thumbnailDataUrl ? (
          <img
            src={liv.thumbnailDataUrl}
            alt={liv.title}
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            <Plane className="h-10 w-10 text-slate-700" />
          </div>
        )}
        {liv.tailNumber && (
          <span className="absolute right-2 top-2 rounded bg-slate-950/80 px-1.5 py-0.5 font-mono text-[10px] font-medium text-slate-100 ring-1 ring-slate-700">
            {liv.tailNumber}
          </span>
        )}
      </div>
      <div className="flex flex-1 flex-col gap-1 p-3">
        <p className="line-clamp-2 text-xs font-medium leading-tight text-slate-100">
          {liv.airline || liv.variation || liv.title}
        </p>
        {liv.variation && liv.airline && liv.variation !== liv.airline && (
          <p className="line-clamp-1 text-[10px] text-slate-500">
            {liv.variation}
          </p>
        )}
        {liv.texture && (
          <p className="mt-auto truncate text-[10px] font-mono text-slate-600">
            {liv.texture}
          </p>
        )}
      </div>
    </li>
  );
}

// ----------------------------------------------------------------------------
// Subcomponentes
// ----------------------------------------------------------------------------

function CardsGrid({
  items,
  updates,
  onOpen,
}: {
  items: { p: CommunityPackage; t: DerivedType }[];
  updates: { folderName: string }[];
  onOpen: (folder: string) => void;
}) {
  return (
    <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
      {items.map(({ p, t }) => (
        <PackageCard
          key={p.folderName}
          pkg={p}
          derived={t}
          hasUpdate={updates.some((u) => u.folderName === p.folderName)}
          onClick={() => onOpen(p.folderName)}
        />
      ))}
    </ul>
  );
}

function TypeChip({
  active,
  onClick,
  icon,
  label,
  count,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
  count: number;
}) {
  return (
    <button
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs transition-colors ${
        active
          ? "border-brand-500 bg-brand-500/15 text-brand-100"
          : "border-slate-800 bg-slate-900/40 text-slate-300 hover:border-slate-700 hover:bg-slate-800/60"
      }`}
    >
      {icon}
      {label}
      <span
        className={`ml-0.5 rounded-full px-1.5 text-[10px] font-medium ${
          active ? "bg-brand-500/30 text-brand-100" : "bg-slate-800 text-slate-400"
        }`}
      >
        {count}
      </span>
    </button>
  );
}

function EmptyState({
  hasAny,
  hasFilter,
}: {
  hasAny: boolean;
  hasFilter: boolean;
}) {
  if (!hasAny) {
    return (
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-12 text-center">
        <Package className="mx-auto mb-3 h-8 w-8 text-slate-700" />
        <p className="text-sm text-slate-400">No hay addons no-escenario instalados.</p>
        <p className="mt-1 text-xs text-slate-600">
          Arrastra un .zip / .rar / .7z / .ptp para instalar uno, o usa
          la pestaña Buscar para descargar desde un catálogo.
        </p>
      </div>
    );
  }
  if (hasFilter) {
    return (
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-12 text-center text-xs text-slate-500">
        Ningún addon coincide con tu filtro.
      </div>
    );
  }
  return null;
}

// ----------------------------------------------------------------------------
// Card individual
// ----------------------------------------------------------------------------

function PackageCard({
  pkg,
  derived,
  hasUpdate,
  onClick,
}: {
  pkg: CommunityPackage;
  derived: DerivedType;
  hasUpdate: boolean;
  onClick: () => void;
}) {
  // Skip de thumbnails para títulos con keywords de placeholder
  // (NTEST, PAINTKIT, TEMPLATE, etc.) — muchos addons dejan el
  // PNG gris "PLACEHOLDER" en esos casos.
  const skipThumb = looksLikePlaceholderTitle(pkg.title);
  const thumb = useThumbnail(pkg.folderName, skipThumb);
  const sizeLabel =
    pkg.sizeBytes != null ? formatStorage(pkg.sizeBytes) : null;
  const { enabled, busy, toggle } = usePackageToggle(pkg);

  return (
    <li>
      {/* div clickable (no <button>) — el footer anida el toggle y el
          botón de metadatos, y HTML no permite button dentro de button. */}
      <div
        onClick={onClick}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => e.key === "Enter" && onClick()}
        className="group flex h-full w-full cursor-pointer flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/50 text-left transition-colors hover:border-brand-500/40 hover:bg-slate-900"
      >
        {/* Thumbnail rectangular arriba (16:9). Cuando el addon está
            apagado, la imagen se apaga con él (grayscale + dim). */}
        <div className="relative aspect-[16/9] w-full shrink-0 overflow-hidden bg-gradient-to-br from-slate-800 to-slate-950">
          {thumb ? (
            <img
              src={thumb}
              alt=""
              loading="lazy"
              decoding="async"
              className={`h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.04] ${
                enabled ? "" : "opacity-40 grayscale"
              }`}
              onError={(e) => {
                (e.currentTarget as HTMLImageElement).style.display = "none";
              }}
            />
          ) : (
            // (v4.26.0) Sin thumbnail → arte tipado (gradiente +
            // iniciales + marca de agua) en vez del fondo vacío.
            <div className={`h-full w-full ${enabled ? "" : "opacity-40 grayscale"}`}>
              <AddonFallbackArt derived={derived} title={pkg.title} />
            </div>
          )}
          {/* Badges superpuestos sobre la imagen — tipo y update. */}
          <div className="pointer-events-none absolute left-2 top-2 flex flex-wrap gap-1">
            <span className={typeBadgeClass(derived)}>
              {typeLabel(derived)}
            </span>
            {hasUpdate && (
              <span className="rounded bg-amber-500 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/30">
                Update
              </span>
            )}
            {!enabled && (
              <span className="rounded bg-slate-900/90 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-slate-400 ring-1 ring-slate-700">
                {t("addons.card.disabled_badge")}
              </span>
            )}
          </div>
        </div>

        <div className="flex flex-1 flex-col p-3">
          <div
            className={`line-clamp-2 text-sm font-medium leading-snug ${
              enabled ? "text-slate-100" : "text-slate-500"
            }`}
          >
            {pkg.title}
          </div>
          <div className="mt-1 truncate text-[11px] text-slate-500">
            {pkg.creator ?? t("dashboard.unknown_author")}
            {pkg.packageVersion && ` · v${pkg.packageVersion}`}
          </div>
          {/* (v4.25.0) Footer estilo My Content: toggle físico a la
              izquierda, tamaño + botón de metadatos a la derecha. */}
          <div className="mt-auto flex items-center gap-2 pt-2.5">
            <ToggleSwitch on={enabled} busy={busy} onToggle={toggle} small />
            <button
              onClick={(e) => {
                e.stopPropagation();
                onClick();
              }}
              title={t("addons.card.edit_title")}
              className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
            >
              <Pencil className="h-3 w-3" />
            </button>
            <span className="ml-auto shrink-0 text-[10px] font-medium tabular-nums text-slate-500">
              {sizeLabel}
            </span>
          </div>
        </div>
      </div>
    </li>
  );
}

// ----------------------------------------------------------------------------
// Helpers de tipo (icono, label, badge color)
// ----------------------------------------------------------------------------

function typeIcon(t: DerivedType, className = "h-3.5 w-3.5"): React.ReactNode {
  switch (t) {
    case "AIRCRAFT":
      return <Plane className={className} />;
    case "LIVERY":
      return <Palette className={className} />;
    case "INSTRUMENT":
      return <Cog className={className} />;
    case "MISC":
      return <Music className={className} />;
    default:
      return <HelpCircle className={className} />;
  }
}

function typeLabel(ty: DerivedType): string {
  switch (ty) {
    case "AIRCRAFT":
      return t("addons.type.aircraft");
    case "LIVERY":
      return t("addons.type.livery");
    case "INSTRUMENT":
      return t("addons.type.instrument");
    case "MISC":
      return t("addons.type.misc");
    default:
      return t("addons.type.unknown");
  }
}

function typeBadgeClass(t: DerivedType): string {
  const base =
    "rounded-md px-1.5 py-0.5 text-[10px] uppercase tracking-wide font-semibold backdrop-blur shadow-md";
  switch (t) {
    case "AIRCRAFT":
      return `${base} bg-sky-500/80 text-sky-50`;
    case "LIVERY":
      return `${base} bg-violet-500/80 text-violet-50`;
    case "INSTRUMENT":
      return `${base} bg-fuchsia-500/80 text-fuchsia-50`;
    case "MISC":
      return `${base} bg-amber-500/80 text-amber-950`;
    default:
      return `${base} bg-slate-700/80 text-slate-200`;
  }
}
