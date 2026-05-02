import { useEffect, useMemo, useState } from "react";
import { Boxes, Filter, Loader2, Package, RefreshCw, Search } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { CommunityPackage } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { PackageDetailModal } from "./PackageDetailModal";
import { derivedType, isAddon, type DerivedType } from "../lib/packageType";
import { api } from "../lib/tauri";

/**
 * Vista paralela al mapa para todo lo que **no** es escenario:
 * AIRCRAFT, LIVERY (derivado), INSTRUMENT, MISC y UNKNOWN.
 *
 * Misma fuente de verdad (`useCommunityStore`), mismo modal de
 * detalle (`PackageDetailModal`). Sin mapa porque los addons no
 * geolocalizan.
 *
 * El filtro por tipo usa `derivedType()` — un único helper
 * compartido con MapView que distingue liveries de aircraft
 * mirando el campo `dependencies` del manifest. Esto evita que la
 * lista mezcle liveries dentro de la categoría AIRCRAFT.
 */
export function AddonsView() {
  const allPackages = useCommunityStore((s) => s.packages);
  const rescan = useCommunityStore((s) => s.rescan);
  const scanning = useCommunityStore((s) => s.scanning);
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

  const knownTypes = useMemo(() => {
    const set = new Set<DerivedType>();
    for (const { t } of addons) set.add(t);
    // Orden lógico: aircraft, livery, instrument, misc, unknown.
    const order: DerivedType[] = [
      "AIRCRAFT",
      "LIVERY",
      "INSTRUMENT",
      "MISC",
      "UNKNOWN",
    ];
    return order.filter((t) => set.has(t));
  }, [addons]);

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

  // Conteos por tipo para mostrar "AIRCRAFT (3)" en el dropdown.
  const counts = useMemo(() => {
    const m = new Map<DerivedType, number>();
    for (const { t } of addons) m.set(t, (m.get(t) ?? 0) + 1);
    return m;
  }, [addons]);

  return (
    <div className="rounded-2xl border border-slate-800 bg-slate-900/40">
      <header className="flex flex-wrap items-center gap-3 border-b border-slate-800 px-5 py-3">
        <div className="flex items-center gap-2">
          <Boxes className="h-4 w-4 text-brand-300" />
          <h2 className="text-sm font-semibold text-slate-100">
            Addons ({addons.length})
          </h2>
        </div>

        <div className="ml-auto flex flex-wrap items-center gap-2">
          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filtrar…"
              className="w-48 rounded-md border border-slate-800 bg-slate-950/50 py-1.5 pl-7 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30"
            />
          </div>
          <div className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-950/50 px-2 py-1.5 text-xs text-slate-300">
            <Filter className="h-3.5 w-3.5 text-slate-500" />
            <select
              value={typeFilter}
              onChange={(e) => setTypeFilter(e.target.value as DerivedType | "ALL")}
              className="bg-transparent text-xs focus:outline-none"
            >
              <option value="ALL">Todos los tipos</option>
              {knownTypes.map((t) => (
                <option key={t} value={t}>
                  {t} ({counts.get(t) ?? 0})
                </option>
              ))}
            </select>
          </div>
          <button
            onClick={rescan}
            disabled={scanning}
            title="Re-escanear Community"
            className="inline-flex items-center gap-1 rounded-md border border-slate-800 px-2 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:opacity-50"
          >
            {scanning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </div>
      </header>

      <div className="max-h-[calc(100vh-19rem)] min-h-[320px] overflow-y-auto">
        {visible.length === 0 ? (
          <div className="px-6 py-12 text-center text-sm text-slate-500">
            {addons.length === 0
              ? "No hay addons no-escenario detectados todavía. Pulsa el botón de re-escaneo o instala algo."
              : "Ningún addon coincide con el filtro."}
          </div>
        ) : (
          <ul className="grid grid-cols-1 gap-2 p-3 md:grid-cols-2">
            {visible.map(({ p, t }) => (
              <PackageCard
                key={p.folderName}
                pkg={p}
                derived={t}
                hasUpdate={updates.some((u) => u.folderName === p.folderName)}
                onClick={() => openDetails(p.folderName)}
              />
            ))}
          </ul>
        )}
      </div>

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

/** Carga lazy del thumbnail del paquete. Devuelve `null` si no hay
 *  thumbnail; `convertFileSrc()` traduce el path absoluto del backend
 *  a un URL compatible con `<img src="...">` en el webview de Tauri. */
function useThumbnail(folderName: string): string | null {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    api
      .packageThumbnail(folderName)
      .then((path) => {
        if (cancelled) return;
        setSrc(path ? convertFileSrc(path) : null);
      })
      .catch(() => {
        if (!cancelled) setSrc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [folderName]);
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
  const thumb = useThumbnail(pkg.folderName);

  return (
    <li>
      <button
        onClick={onClick}
        className="group flex w-full items-stretch gap-0 overflow-hidden rounded-xl border border-slate-800 bg-slate-900/50 text-left transition-colors hover:border-brand-500/40 hover:bg-slate-900"
      >
        {/* Imagen lateral — cuando el paquete trae un thumbnail
            (jpg/png) lo mostramos. Si no, fallback al icono de
            Package en un fondo gradiente para que la card no se vea
            hueca. Ratio fijo 1:1 mantiene la cuadrícula alineada. */}
        <div className="relative aspect-square w-24 shrink-0 overflow-hidden bg-gradient-to-br from-slate-800 via-slate-850 to-slate-900">
          {thumb ? (
            <img
              src={thumb}
              alt=""
              loading="lazy"
              decoding="async"
              className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.04]"
              onError={(e) => {
                // Si la imagen falla, ocultarla y mostrar el icon.
                (e.currentTarget as HTMLImageElement).style.display = "none";
              }}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-slate-700">
              <Package className="h-7 w-7" strokeWidth={1.5} />
            </div>
          )}
        </div>
        <div className="flex min-w-0 flex-1 flex-col justify-center p-3">
          <div className="flex items-center gap-1.5">
            <span className={typeBadgeClass(derived)}>{derived}</span>
            {hasUpdate && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300 ring-1 ring-amber-500/30">
                Update
              </span>
            )}
          </div>
          <div className="mt-1 truncate text-sm font-medium text-slate-100">
            {pkg.title}
          </div>
          <div className="mt-0.5 truncate text-[11px] text-slate-500">
            {pkg.creator ?? "Autor desconocido"}
            {pkg.packageVersion && ` · v${pkg.packageVersion}`}
          </div>
        </div>
      </button>
    </li>
  );
}

function typeBadgeClass(t: DerivedType): string {
  const base = "rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wide font-semibold";
  switch (t) {
    case "AIRCRAFT":
      return `${base} bg-sky-500/15 text-sky-300 ring-1 ring-sky-500/30`;
    case "LIVERY":
      return `${base} bg-violet-500/15 text-violet-300 ring-1 ring-violet-500/30`;
    case "INSTRUMENT":
      return `${base} bg-fuchsia-500/15 text-fuchsia-300 ring-1 ring-fuchsia-500/30`;
    case "MISC":
      return `${base} bg-slate-700/60 text-slate-300 ring-1 ring-slate-600`;
    default:
      return `${base} bg-slate-800 text-slate-400 ring-1 ring-slate-700`;
  }
}
