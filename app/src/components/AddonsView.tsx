import { useEffect, useMemo, useState } from "react";
import {
  Boxes,
  Cog,
  Music,
  Package,
  Plane,
  Palette,
  Search,
  HelpCircle,
} from "lucide-react";
import type { CommunityPackage, PmdgLivery } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { PackageDetailModal } from "./PackageDetailModal";
import {
  derivedType,
  isAddon,
  looksLikePlaceholderTitle,
  type DerivedType,
} from "../lib/packageType";
import { api } from "../lib/tauri";

/**
 * Vista de addons no-escenario: aviones, liveries, instrumentos,
 * misc, unknown.
 *
 * Layout actualizado:
 *   · Header con search global + filtros como **chips** (no
 *     dropdown). Cada chip muestra el icono + label + contador.
 *   · Cuerpo: cuando no hay filtro de tipo, agrupamos por
 *     categoría con cabeceras stickies. Cuando hay filtro de tipo
 *     activo, lista plana sin cabeceras.
 *   · Cards en grid 1/2/3/4 columnas según ancho de pantalla.
 *     Cards más limpios — thumbnail rectangular arriba (no cuadrada
 *     a la izquierda) con el contenido debajo.
 */
export function AddonsView() {
  const allPackages = useCommunityStore((s) => s.packages);
  const detailsFor = useCommunityStore((s) => s.detailsFor);
  const openDetails = useCommunityStore((s) => s.openDetails);
  const updates = useCommunityStore((s) => s.updates);

  const [filter, setFilter] = useState("");
  const [typeFilter, setTypeFilter] = useState<DerivedType | "ALL">("ALL");

  // Excluimos escenarios — son del MapView. Lo que queda son
  // AIRCRAFT, LIVERY (derivado), INSTRUMENT, MISC, UNKNOWN.
  const addons = useMemo(
    () => allPackages.filter(isAddon).map((p) => ({ p, t: derivedType(p) })),
    [allPackages],
  );

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

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return addons.filter(({ p, t }) => {
      if (typeFilter !== "ALL" && t !== typeFilter) return false;
      if (!q) return true;
      return [p.title, p.creator, p.folderName]
        .filter(Boolean)
        .some((s) => s!.toLowerCase().includes(q));
    });
  }, [addons, filter, typeFilter]);

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
      {/* Header — título + search + chips de filtro de tipo. */}
      <header className="flex flex-wrap items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-3">
        <div className="flex items-center gap-2">
          <Boxes className="h-4 w-4 text-brand-300" />
          <h2 className="text-sm font-semibold text-slate-100">
            Mis addons
          </h2>
          <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-400">
            {addons.length}
          </span>
        </div>

        <div className="ml-auto flex flex-1 flex-wrap items-center gap-2 md:flex-none">
          <div className="relative flex-1 md:flex-none">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Buscar por título, autor, carpeta…"
              className="w-full rounded-md border border-slate-800 bg-slate-950/50 py-1.5 pl-8 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30 md:w-72"
            />
          </div>
        </div>
      </header>

      {/* Chips de tipo — fila con conteos visibles. */}
      <div className="flex flex-wrap gap-1.5">
        <TypeChip
          active={typeFilter === "ALL"}
          onClick={() => setTypeFilter("ALL")}
          icon={<Package className="h-3.5 w-3.5" />}
          label="Todos"
          count={addons.length}
        />
        {presentTypes.map((t) => (
          <TypeChip
            key={t}
            active={typeFilter === t}
            onClick={() => setTypeFilter(t)}
            icon={typeIcon(t)}
            label={typeLabel(t)}
            count={counts.get(t) ?? 0}
          />
        ))}
      </div>

      {/* Cuerpo */}
      {visible.length === 0 ? (
        <EmptyState hasAny={addons.length > 0} hasFilter={!!filter || typeFilter !== "ALL"} />
      ) : typeFilter !== "ALL" || filter.trim() !== "" ? (
        // Lista plana — el usuario ya filtró, no agrupamos.
        <CardsGrid
          items={visible}
          updates={updates}
          onOpen={openDetails}
        />
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

      {/* (v3.0.0 / v3.1.0) Sección Aircraft Liveries — escaneo
          dedicado de paquetes `*-aircraft-*` (PMDG + Fenix +
          iniBuilds + FlyByWire…) con parsing del aircraft.cfg.
          Recibe el filtro de búsqueda global del header para que las
          liveries también aparezcan/desaparezcan al buscar. */}
      <AircraftLiveriesSection filter={filter} />

      {detailsPkg && (
        <PackageDetailModal
          pkg={detailsPkg}
          update={detailsUpdate}
          onClose={() => openDetails(null)}
        />
      )}
    </div>
  );
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
  const [expanded, setExpanded] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .listPmdgLiveries()
      .then((data) => {
        if (cancelled) return;
        setLiveries(data);
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

  // (v3.1.0) Aplica el filtro global del header sobre los liveries
  // — antes el buscador sólo indexaba la raíz y el usuario reportó
  // "Ningún addon coincide con tu filtro" al buscar una librea PMDG
  // específica. Ahora también matcheamos tail number, airline,
  // variation, vendor y package folder.
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
  // Si hay filtro activo y no matchea nada, no renderizamos la
  // sección — la EmptyState global del addons view ya maneja eso.
  if (q && filtered.length === 0) {
    return null;
  }

  // Group by vendor + model.
  type Key = string; // "vendor::model"
  const byKey = new Map<Key, PmdgLivery[]>();
  for (const l of filtered) {
    const key = `${l.vendor}::${l.model}`;
    const arr = byKey.get(key) ?? [];
    arr.push(l);
    byKey.set(key, arr);
  }

  const totalCount = filtered.length;

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/40">
      <header
        className="flex cursor-pointer items-center gap-2 border-b border-slate-800 px-4 py-3 hover:bg-slate-900/60"
        onClick={() => setExpanded((v) => !v)}
      >
        <Plane className="h-4 w-4 text-amber-300" />
        <h3 className="text-sm font-semibold text-slate-100">
          Aircraft Liveries
        </h3>
        <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-400">
          {totalCount}
        </span>
        <span className="ml-auto text-[11px] text-slate-500">
          {expanded ? "↓ Colapsar" : "→ Expandir"}
        </span>
      </header>
      {expanded && (
        <div className="space-y-6 p-4">
          {Array.from(byKey.entries())
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([key, items]) => {
              const [vendor, model] = key.split("::");
              const v = vendorLabel(vendor);
              return (
                <div key={key}>
                  <h4 className="mb-3 inline-flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                    <span className={v.tone}>{v.name}</span>
                    <span>{modelLabel(vendor, model)}</span>
                    <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium text-slate-500 normal-case tracking-normal">
                      {items.length}
                    </span>
                  </h4>
                  {/* (v3.1.0) Grid homogéneo con el resto de los
                      paquetes de Addons: 1/2/3/4 columnas según
                      ancho. Cards más altas con thumbnail rectangular
                      arriba — mismo patrón que `PackageCard`. */}
                  <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                    {items.map((liv, idx) => (
                      <LiveryCard
                        key={`${liv.title}-${idx}`}
                        liv={liv}
                      />
                    ))}
                  </ul>
                </div>
              );
            })}
        </div>
      )}
    </section>
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

const thumbnailCache = new Map<string, string | null>();

function useThumbnail(folderName: string, skip: boolean = false): string | null {
  const cached = thumbnailCache.get(folderName);
  const [src, setSrc] = useState<string | null>(cached ?? null);
  useEffect(() => {
    // Si el caller dice "skip" (título de placeholder/test), ni
    // siquiera intentamos cargar el thumbnail. Cacheamos null para
    // evitar re-intentos.
    if (skip) {
      thumbnailCache.set(folderName, null);
      setSrc(null);
      return;
    }
    if (thumbnailCache.has(folderName)) {
      setSrc(thumbnailCache.get(folderName) ?? null);
      return;
    }
    let cancelled = false;
    api
      .packageThumbnail(folderName)
      .then((dataUrl) => {
        if (cancelled) return;
        // Heurística anti-placeholder: si el data URL es muy chico
        // (<3 KB de base64 ≈ <2 KB de imagen real), probablemente
        // es un PNG genérico "PLACEHOLDER" que el dev dejó como
        // marcador. Cacheamos null y renderemos el icono de
        // categoría en su lugar.
        const looksTiny = dataUrl !== null && dataUrl.length < 3000;
        const finalUrl = looksTiny ? null : dataUrl;
        thumbnailCache.set(folderName, finalUrl);
        setSrc(finalUrl);
      })
      .catch(() => {
        if (cancelled) return;
        thumbnailCache.set(folderName, null);
        setSrc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [folderName, skip]);
  return src;
}

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
  const sizeMb =
    pkg.sizeBytes != null ? (pkg.sizeBytes / 1_000_000).toFixed(0) : null;

  return (
    <li>
      <button
        onClick={onClick}
        className="group flex h-full w-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/50 text-left transition-colors hover:border-brand-500/40 hover:bg-slate-900"
      >
        {/* Thumbnail rectangular arriba (16:9). Más espacio que el
            cuadrado lateral del diseño anterior, mejor para fotos
            de aviones que son apaisadas. */}
        <div className="relative aspect-[16/9] w-full shrink-0 overflow-hidden bg-gradient-to-br from-slate-800 to-slate-950">
          {thumb ? (
            <img
              src={thumb}
              alt=""
              loading="lazy"
              decoding="async"
              className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.04]"
              onError={(e) => {
                (e.currentTarget as HTMLImageElement).style.display = "none";
              }}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-slate-700">
              {typeIcon(derived, "h-8 w-8")}
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
          </div>
        </div>

        <div className="flex flex-1 flex-col p-3">
          <div className="line-clamp-2 text-sm font-medium leading-snug text-slate-100">
            {pkg.title}
          </div>
          <div className="mt-1 truncate text-[11px] text-slate-500">
            {pkg.creator ?? "Autor desconocido"}
          </div>
          {/* Footer con metadata: versión + tamaño */}
          <div className="mt-auto flex items-center justify-between gap-2 pt-2 text-[10px] text-slate-500">
            {pkg.packageVersion ? (
              <span className="truncate font-mono text-slate-400">
                v{pkg.packageVersion}
              </span>
            ) : (
              <span />
            )}
            {sizeMb && (
              <span className="shrink-0 tabular-nums text-slate-500">
                {sizeMb} MB
              </span>
            )}
          </div>
        </div>
      </button>
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

function typeLabel(t: DerivedType): string {
  switch (t) {
    case "AIRCRAFT":
      return "Aviones";
    case "LIVERY":
      return "Liveries";
    case "INSTRUMENT":
      return "Instrumentos";
    case "MISC":
      return "Sonido / Misc";
    default:
      return "Sin clasificar";
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
