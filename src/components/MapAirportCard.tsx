import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  ChevronDown,
  Folder,
  Loader2,
  MapPin,
  RefreshCcw,
  Trash2,
  X,
} from "lucide-react";
import type {
  Addon,
  AvailableUpdate,
  CommunityPackage,
  DownloadMethod,
  GsxAirportParkings,
  GsxParking,
  GsxParkingKind,
} from "../lib/types";
import { GSX_KIND_COLORS } from "../lib/gsxKinds";
import { airlineChips, gateAirlines, gateKey } from "../lib/gsxAirlines";
import { AirlineLogo } from "./AirlineLogo";
import { api } from "../lib/tauri";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { useToastStore } from "../stores/useToastStore";
import { useThumbnail } from "../lib/thumbnails";
import {
  Detail,
  GsxProfileRow,
  RepairMethodPicker,
  formatBytes,
} from "./PackageDetailModal";
import { looksLikePlaceholderTitle } from "../lib/packageType";
import { t } from "../lib/i18n";

/**
 * (v4.25.0) Card contextual del aeropuerto seleccionado en el Map
 * (scenery). Sustituye al modal emergente: fusiona la cabecera con
 * preview de imagen + un acordeón "Ficha técnica" con las filas del
 * viejo modal (Folder, Version, Size, Min MSFS, Last modified, GSX) +
 * las 3 acciones críticas SIEMPRE visibles abajo:
 *   Open Folder (izquierda) · Reinstall (centro) · Uninstall (rojo).
 *
 * Se ancla flotando sobre el mapa (esquina superior derecha) cuando el
 * usuario selecciona un aeropuerto desde la sidebar o clickeando el
 * marker. Cerrar = deseleccionar.
 */
export function MapAirportCard({
  pkg,
  update,
  onClose,
  gsxParkings,
  pickedParking,
  onPickParking,
  onGsxVisibleChange,
}: {
  pkg: CommunityPackage;
  update: AvailableUpdate | null;
  onClose: () => void;
  /** (19/10) Gates/parkings del perfil GSX (null = sin perfil o cargando). */
  gsxParkings: GsxAirportParkings | null;
  pickedParking: string | null;
  onPickParking: (name: string | null) => void;
  /** (26/10) Gates del chip de aerolínea elegido (`null` = todos) → pines. */
  onGsxVisibleChange?: (names: Set<string> | null) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState<"none" | "uninstalling" | "repairing">("none");
  const [confirmingUninstall, setConfirmingUninstall] = useState(false);
  const [pickingRepair, setPickingRepair] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [matches, setMatches] = useState<Addon[]>([]);
  const [searching, setSearching] = useState(false);

  const rescan = useCommunityStore((s) => s.rescan);
  const startDownload = useDownloadsStore((s) => s.start);
  const pushToast = useToastStore((s) => s.push);

  const skipThumb = looksLikePlaceholderTitle(pkg.title);
  const thumb = useThumbnail(pkg.folderName, skipThumb);

  // Misma estrategia de búsqueda que el modal: ICAO en ambas fuentes +
  // keyword del título en Simplaza.
  const searchKeyword = useMemo(() => {
    const words = pkg.title
      .split(/[\s\-_]+/)
      .map((w) => w.replace(/[^A-Za-z0-9]/g, ""))
      .filter((w) => w.length >= 3);
    return words.slice(0, 3).join(" ").trim();
  }, [pkg.title]);

  const loadCatalogIfNeeded = async () => {
    if (matches.length > 0 || searching) return;
    setSearching(true);
    try {
      const queries: Promise<Addon[]>[] = [];
      if (pkg.icao) {
        queries.push(api.search(pkg.icao, "sceneryaddons").catch(() => [] as Addon[]));
        queries.push(api.search(pkg.icao, "simplaza").catch(() => [] as Addon[]));
      }
      if (searchKeyword) {
        queries.push(api.search(searchKeyword, "simplaza").catch(() => [] as Addon[]));
      }
      const results = await Promise.all(queries);
      const seen = new Set<string>();
      const merged: Addon[] = [];
      for (const list of results) {
        for (const a of list) {
          if (seen.has(a.id)) continue;
          seen.add(a.id);
          merged.push(a);
        }
      }
      setMatches(merged);
    } finally {
      setSearching(false);
    }
  };

  const doUninstall = async () => {
    setBusy("uninstalling");
    setError(null);
    try {
      const report = await api.uninstallCommunityPackage(pkg.folderName);
      await rescan();
      const removed = report.removedPaths.length;
      pushToast({
        kind: removed > 0 ? "success" : "info",
        title:
          removed === 0
            ? t("pkg.not_on_disk")
            : removed === 1
              ? t("pkg.uninstalled_one")
              : t("pkg.uninstalled_many", { n: removed }),
        message:
          removed > 0
            ? report.removedPaths.join(" · ")
            : t("pkg.uninstalled_db_only"),
      });
      if (report.failedPaths.length > 0) {
        pushToast({
          kind: "error",
          title: t("pkg.uninstall_failed", { n: report.failedPaths.length }),
          message: report.failedPaths
            .map((f) => `${f.path}: ${f.error}`)
            .join(" · "),
        });
      }
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy("none");
    }
  };

  const doRepairWith = async (addon: Addon, method: DownloadMethod) => {
    setBusy("repairing");
    setPickingRepair(false);
    setError(null);
    try {
      await api.uninstallCommunityPackage(pkg.folderName);
      await startDownload({
        addonId: addon.id,
        addonTitle: addon.name,
        source: addon.source,
        method,
        addonSimulator: addon.simulator,
      });
      await rescan();
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy("none");
    }
  };

  const openRepairPicker = () => {
    setPickingRepair(true);
    void loadCatalogIfNeeded();
  };

  const canRepair = !!pkg.icao || !!searchKeyword;

  return (
    <motion.div
      key={pkg.folderName}
      initial={{ opacity: 0, x: 16, scale: 0.98 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, x: 16, scale: 0.98 }}
      transition={{ duration: 0.18 }}
      className="absolute right-3 top-3 z-20 flex max-h-[calc(100%-1.5rem)] w-[330px] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-bg/95 shadow-2xl ring-1 ring-sf-border backdrop-blur"
    >
      {/* Cabecera con preview — se mantiene del popover original. */}
      <div className="relative h-32 w-full shrink-0 overflow-hidden bg-gradient-to-br from-slate-800 to-slate-950">
        {thumb ? (
          <img
            src={thumb}
            alt=""
            className="h-full w-full object-cover"
            onError={(e) => {
              (e.currentTarget as HTMLImageElement).style.display = "none";
            }}
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-sf-muted/70">
            <MapPin className="h-10 w-10" />
          </div>
        )}
        <button
          onClick={onClose}
          disabled={busy !== "none"}
          className="absolute right-2 top-2 rounded-md bg-slate-950/70 p-1 text-sf-text/90 backdrop-blur hover:bg-sf-surface hover:text-sf-text disabled:opacity-50"
        >
          <X className="h-3.5 w-3.5" />
        </button>
        <div className="pointer-events-none absolute bottom-2 left-2 flex flex-wrap gap-1">
          {pkg.icao && (
            <span className="rounded bg-emerald-500/90 px-1.5 py-0.5 font-mono text-[10px] font-bold text-emerald-950 shadow">
              {pkg.icao}
            </span>
          )}
          {pkg.contentType && (
            <span className="rounded bg-slate-950/80 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-sf-text/90 ring-1 ring-sf-border">
              {pkg.contentType}
            </span>
          )}
          {update && (
            <span className="rounded bg-amber-500/90 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-amber-950 shadow">
              Update {update.installedVersion} → {update.latestVersion}
            </span>
          )}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="px-4 pb-1 pt-3">
          {/* (v4.31.0) Nombre real del aeropuerto como título (los
              packs DominicDesignTeam traen title genérico "Airport"). */}
          <h3
            className="truncate text-sm font-semibold text-sf-text"
            title={pkg.airportName ?? pkg.title}
          >
            {pkg.airportName ?? pkg.title}
          </h3>
          <p className="mt-0.5 truncate text-[11px] text-sf-muted">
            {pkg.creator ?? "—"}
            {pkg.packageVersion && ` · v${pkg.packageVersion}`}
          </p>
        </div>

        {/* Acordeón "Ficha técnica" — las filas del viejo modal. */}
        <div className="px-4 pb-3 pt-2">
          <button
            onClick={() => setExpanded((v) => !v)}
            className="flex w-full items-center justify-between rounded-lg border border-sf-border bg-sf-surface px-3 py-2 text-xs font-medium text-sf-text transition-colors hover:border-brand-500/40 hover:bg-sf-surface"
          >
            {t("map.card.tech_info")}
            <ChevronDown
              className={`h-3.5 w-3.5 text-sf-muted transition-transform ${
                expanded ? "rotate-180" : ""
              }`}
            />
          </button>
          <AnimatePresence initial={false}>
            {expanded && (
              <motion.div
                key="tech"
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.18 }}
                className="overflow-hidden"
              >
                <dl className="grid grid-cols-2 gap-x-3 gap-y-1.5 px-1 pt-3 text-xs">
                  <Detail label={t("pkg.folder")} value={pkg.folderName} mono />
                  <Detail label={t("pkg.version")} value={pkg.packageVersion ?? "—"} />
                  <Detail
                    label={t("pkg.size")}
                    value={pkg.sizeBytes != null ? formatBytes(pkg.sizeBytes) : "—"}
                  />
                  <Detail
                    label={t("pkg.min_msfs")}
                    value={pkg.minimumGameVersion ?? "—"}
                  />
                  {pkg.airportName && (
                    <Detail label={t("pkg.airport")} value={pkg.airportName} />
                  )}
                  <Detail
                    label={t("pkg.last_modified")}
                    value={pkg.folderModifiedAt ?? "—"}
                  />
                  <GsxProfileRow icao={pkg.icao} />
                </dl>
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        {/* (19/10) Gates y parkings del perfil GSX instalado. */}
        {gsxParkings && (
          <GsxParkingList
            data={gsxParkings}
            picked={pickedParking}
            onPick={onPickParking}
            onVisibleChange={onGsxVisibleChange}
          />
        )}

        {error && (
          <div className="mx-4 mb-2 flex items-start gap-2 rounded-lg bg-rose-500/15 px-3 py-2 text-xs text-rose-200 ring-1 ring-rose-500/30">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="flex-1">{error}</span>
          </div>
        )}

        {confirmingUninstall && (
          <div className="mx-4 mb-3 rounded-lg border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-100">
            <p className="font-medium">
              {t("pkg.uninstall_confirm.q")}{" "}
              <span className="font-mono">{pkg.folderName}</span>?
            </p>
            <p className="mt-1 text-rose-200/80">{t("pkg.uninstall_confirm.body")}</p>
            <div className="mt-2 flex justify-end gap-2">
              <button
                onClick={() => setConfirmingUninstall(false)}
                className="rounded-md border border-sf-border px-3 py-1 text-sf-text hover:bg-sf-raised"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void doUninstall()}
                disabled={busy !== "none"}
                className="inline-flex items-center gap-1 rounded-md bg-rose-500 px-3 py-1 font-medium text-rose-950 hover:bg-rose-400 disabled:opacity-50"
              >
                {busy === "uninstalling" ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <Trash2 className="h-3 w-3" />
                )}
                {t("common.confirm")}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Acciones críticas SIEMPRE visibles — nunca dentro del acordeón.
          Open Folder (izq) · Reinstall (centro) · Uninstall (rojo, der). */}
      <footer className="flex shrink-0 items-center gap-1.5 border-t border-sf-border px-3 py-2.5">
        <button
          onClick={() => void api.openLocalPath(pkg.installPath)}
          disabled={busy !== "none"}
          className="inline-flex items-center gap-1 rounded-lg border border-sf-border bg-sf-raised px-2.5 py-1.5 text-[11px] font-medium text-sf-text hover:border-brand-500/40 hover:bg-sf-raised disabled:opacity-50"
        >
          <Folder className="h-3 w-3" />
          {t("pkg.open_folder")}
        </button>
        <button
          onClick={openRepairPicker}
          disabled={busy !== "none" || !canRepair}
          title={
            canRepair
              ? update
                ? t("pkg.update_to", { v: update.latestVersion })
                : t("pkg.reinstall_picker")
              : t("pkg.cannot_reinstall")
          }
          className={`inline-flex items-center gap-1 rounded-lg border px-2.5 py-1.5 text-[11px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
            update
              ? "border-amber-400 bg-amber-500 text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
              : "border-amber-500/40 bg-amber-500/10 text-amber-200 hover:border-amber-400 hover:bg-amber-500/20"
          }`}
        >
          {busy === "repairing" ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCcw className="h-3 w-3" />
          )}
          {t("pkg.reinstall")}
        </button>
        <button
          onClick={() => setConfirmingUninstall(true)}
          disabled={busy !== "none"}
          className="ml-auto inline-flex items-center gap-1 rounded-lg border border-rose-500/40 bg-rose-500/10 px-2.5 py-1.5 text-[11px] font-medium text-rose-200 hover:border-rose-400 hover:bg-rose-500/20 disabled:opacity-50"
        >
          <Trash2 className="h-3 w-3" />
          {t("pkg.uninstall")}
        </button>
      </footer>

      {/* Picker de método para Reinstall — overlay sobre la card. */}
      <AnimatePresence>
        {pickingRepair && (
          <RepairMethodPicker
            pkg={pkg}
            matches={matches}
            searching={searching}
            onPick={doRepairWith}
            onCancel={() => setPickingRepair(false)}
          />
        )}
      </AnimatePresence>

    </motion.div>
  );
}

// ----------------------------------------------------------------------------
// (19/10) Lista de gates/parkings del perfil GSX
// (26/10) Por aerolínea: sin filtro, chips con las aerolíneas del perfil
// (`airlinecodes`); con el filtro de aerolínea del FlightBook, sus gates y los
// libres, o —si el perfil no le asigna ninguno— los que usaste con ella.
// ----------------------------------------------------------------------------

const GSX_KIND_ORDER: GsxParkingKind[] = [
  "gate",
  "parking",
  "ramp",
  "cargo",
  "military",
  "pad",
  "other",
];

/** (26/10) Filtro de aerolínea del FlightBook aplicado a los gates. */
export interface GsxListAirline {
  /** `canonicalAirlineKey`, la misma clave que la lista de vuelos. */
  key: string;
  icao: string | null;
  name: string;
  /** Gates que usaste con ella en este aeropuerto (de tus vuelos). */
  used: { name: string; count: number }[];
}

interface GsxSection {
  id: "all" | "mine" | "used" | "free";
  title: string | null;
  parkings: GsxParking[];
  /** Gates de tus vuelos que el perfil no tiene (no se pueden enfocar). */
  missing: { name: string; count: number }[];
}

/** Valor del chip «Sin asignar». */
const FREE_CHIP = "__free";

/**
 * (26/10) Qué gates enseña la lista: secciones, aviso y los visibles. `chip`
 * sólo cuenta sin filtro de aerolínea (`null` = todas).
 */
export function gsxSections(
  data: GsxAirportParkings,
  airline: GsxListAirline | null,
  chip: string | null,
): { sections: GsxSection[]; notice: string | null; visible: GsxParking[] } {
  const all = data.parkings;
  const keysOf = new Map(all.map((p) => [p.name, gateAirlines(p).map((a) => a.key)]));
  const free = all.filter((p) => keysOf.get(p.name)!.length === 0);
  const hasCodes = free.length < all.length;
  let sections: GsxSection[];
  let notice: string | null = null;
  if (airline) {
    const mine = all.filter((p) => keysOf.get(p.name)!.includes(airline.key));
    if (mine.length > 0) {
      sections = [
        { id: "mine", title: t("map.gsx.airline_gates", { name: airline.name }), parkings: mine, missing: [] },
        { id: "free", title: t("map.gsx.unassigned_title"), parkings: free, missing: [] },
      ];
    } else {
      const byKey = new Map(all.map((p) => [gateKey(p.name), p]));
      const used: GsxParking[] = [];
      const missing: { name: string; count: number }[] = [];
      for (const u of airline.used) {
        const p = byKey.get(gateKey(u.name));
        if (p) used.push(p);
        else missing.push(u);
      }
      const usedNames = new Set(used.map((p) => p.name));
      notice = hasCodes
        ? t("map.gsx.no_airline_gates", { name: airline.name })
        : t("map.gsx.no_codes");
      sections = [
        { id: "used", title: t("map.gsx.used_with", { name: airline.name }), parkings: used, missing },
        {
          id: "free",
          title: hasCodes ? t("map.gsx.unassigned_title") : t("map.gsx.all_title"),
          parkings: free.filter((p) => !usedNames.has(p.name)),
          missing: [],
        },
      ];
    }
  } else if (chip === FREE_CHIP) {
    sections = [{ id: "free", title: null, parkings: free, missing: [] }];
  } else if (chip) {
    sections = [
      { id: "all", title: null, parkings: all.filter((p) => keysOf.get(p.name)!.includes(chip)), missing: [] },
    ];
  } else {
    sections = [{ id: "all", title: null, parkings: all, missing: [] }];
  }
  sections = sections.filter((s) => s.parkings.length > 0 || s.missing.length > 0);
  const seen = new Set<string>();
  const visible = sections.flatMap((s) => s.parkings).filter((p) => !seen.has(p.name) && !!seen.add(p.name));
  return { sections, notice, visible };
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function byKind(parkings: GsxParking[], q: string) {
  const groups = new Map<GsxParkingKind, string[]>();
  for (const p of parkings) {
    if (q && !p.name.toLowerCase().includes(q)) continue;
    const list = groups.get(p.kind) ?? [];
    list.push(p.name);
    groups.set(p.kind, list);
  }
  return GSX_KIND_ORDER.filter((k) => groups.has(k)).map((k) => ({
    kind: k,
    names: groups.get(k)!.sort(collator.compare),
  }));
}

export function GsxParkingList({
  data,
  picked,
  onPick,
  airline = null,
  onVisibleChange,
}: {
  data: GsxAirportParkings;
  picked: string | null;
  onPick: (name: string | null) => void;
  /** (26/10) Filtro de aerolínea del FlightBook. */
  airline?: GsxListAirline | null;
  /** (26/10) Scenery map: nombres visibles con el chip elegido (`null` = todos). */
  onVisibleChange?: (names: Set<string> | null) => void;
}) {
  const [query, setQuery] = useState("");
  const [chip, setChip] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const pickedParking = picked ? data.parkings.find((p) => p.name === picked) : undefined;
  useEffect(() => setChip(null), [data]);
  const chips = useMemo(() => (airline ? [] : airlineChips(data.parkings)), [data, airline]);
  const { sections, notice, visible } = useMemo(
    () => gsxSections(data, airline, airline ? null : chip),
    [data, airline, chip],
  );
  const freeCount = useMemo(
    () => data.parkings.filter((p) => gateAirlines(p).length === 0).length,
    [data],
  );
  // Veces que usaste cada gate con la aerolínea filtrada (marca ×n).
  const usedCount = useMemo(
    () => new Map((airline?.used ?? []).map((u) => [gateKey(u.name), u.count])),
    [airline],
  );
  const filtered = !!airline || chip !== null;
  const visibleKey = filtered ? visible.map((p) => p.name).join("|") : "";
  useEffect(() => {
    onVisibleChange?.(filtered ? new Set(visible.map((p) => p.name)) : null);
    // `visibleKey` resume `visible`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visibleKey, filtered]);
  // (19/10) Al elegir un gate (en la lista o con click en su pin del mapa):
  // si la búsqueda o el chip lo ocultan se quitan, y la lista baja hasta él.
  useEffect(() => {
    if (!picked) return;
    if (query.trim() && !picked.toLowerCase().includes(query.trim().toLowerCase())) {
      setQuery("");
    }
    if (chip !== null && !visible.some((p) => p.name === picked)) setChip(null);
    requestAnimationFrame(() =>
      listRef.current
        ?.querySelector(`[data-gsx-name="${CSS.escape(picked)}"]`)
        ?.scrollIntoView({ block: "nearest", behavior: "smooth" }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [picked]);
  const q = query.trim().toLowerCase();
  const rendered = sections
    .map((s) => ({ ...s, groups: byKind(s.parkings, q) }))
    .filter((s) => s.groups.length > 0 || (s.missing.length > 0 && !q));
  return (
    <div className="px-4 pb-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-sf-text">
          {t("map.gsx.title", { n: filtered ? visible.length : data.parkings.length })}
        </span>
        <span className="truncate text-[10px] text-sf-muted" title={data.fileName}>
          {data.fileName}
        </span>
      </div>
      {chips.length > 0 && (
        <div
          onWheel={(e) => {
            if (e.deltaY !== 0) e.currentTarget.scrollLeft += e.deltaY;
          }}
          className="mb-2 flex gap-1 overflow-x-auto pb-1 [scrollbar-width:thin]"
        >
          <AirlineChip active={chip === null} onClick={() => setChip(null)}>
            {t("map.gsx.all_chip")} <span className="text-sf-muted">{data.parkings.length}</span>
          </AirlineChip>
          {chips.map((c) => (
            <AirlineChip
              key={c.key}
              active={chip === c.key}
              title={c.name}
              onClick={() => setChip(chip === c.key ? null : c.key)}
            >
              <AirlineLogo icao={c.icao} name={c.name} size={14} />
              <span className="font-mono font-semibold">{c.icao}</span>
              <span className="text-sf-muted">{c.count}</span>
            </AirlineChip>
          ))}
          {freeCount > 0 && (
            <AirlineChip
              active={chip === FREE_CHIP}
              title={t("map.gsx.unassigned_hint")}
              onClick={() => setChip(chip === FREE_CHIP ? null : FREE_CHIP)}
            >
              {t("map.gsx.unassigned_chip")} <span className="text-sf-muted">{freeCount}</span>
            </AirlineChip>
          )}
        </div>
      )}
      {notice && (
        <p className="mb-2 rounded-lg bg-amber-500/10 px-2.5 py-1.5 text-[11px] leading-snug text-amber-200 ring-1 ring-amber-500/25">
          {notice}
        </p>
      )}
      {pickedParking && (
        <div className="mb-2 flex items-center gap-2.5 rounded-lg border border-brand-500/40 bg-brand-500/10 px-2.5 py-2">
          <span
            className="h-2.5 w-2.5 shrink-0 rounded-full"
            style={{ backgroundColor: GSX_KIND_COLORS[pickedParking.kind] }}
          />
          <div className="min-w-0 flex-1">
            <div className="truncate font-mono text-sm font-semibold text-sf-text">
              {pickedParking.name}
            </div>
            <div className="truncate text-[10px] text-sf-muted">
              {t(`map.gsx.one.${pickedParking.kind}`)} · {pickedParking.lat.toFixed(5)},{" "}
              {pickedParking.lon.toFixed(5)}
            </div>
            {gateAirlines(pickedParking).length > 0 && (
              <div className="mt-1 flex flex-wrap items-center gap-1">
                {gateAirlines(pickedParking).map((a) => (
                  <span key={a.key} title={a.name} className="inline-flex items-center gap-1 text-[10px] text-sf-text/80">
                    <AirlineLogo icao={a.icao} name={a.name} size={12} />
                    {a.icao}
                  </span>
                ))}
              </div>
            )}
          </div>
          <button
            onClick={() => onPick(null)}
            title={t("map.gsx.unpick")}
            aria-label={t("map.gsx.unpick")}
            className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
      {visible.length > 12 && (
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("map.gsx.search")}
          className="mb-2 w-full rounded-md border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-text placeholder:text-sf-muted focus:border-brand-500/60 focus:outline-none"
        />
      )}
      <div ref={listRef} className="max-h-56 space-y-3 overflow-y-auto pr-1">
        {rendered.map((s) => (
          <div key={s.id} className="space-y-2">
            {s.title && (
              <div className="flex items-center gap-1.5 border-b border-sf-border/60 pb-1 text-[11px] font-semibold text-brand-200">
                {s.id === "mine" || s.id === "used" ? (
                  <AirlineLogo icao={airline?.icao} name={airline?.name} size={13} />
                ) : null}
                <span className="truncate">{s.title}</span>
                <span className="text-sf-muted">· {s.parkings.length + s.missing.length}</span>
              </div>
            )}
            {s.groups.map((g) => (
              <div key={g.kind}>
                <div className="mb-1 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">
                  <span
                    className="h-2 w-2 rounded-full"
                    style={{ backgroundColor: GSX_KIND_COLORS[g.kind] }}
                  />
                  {t(`map.gsx.kind.${g.kind}`)} · {g.names.length}
                </div>
                <div className="flex flex-wrap gap-1">
                  {g.names.map((n) => {
                    const times = usedCount.get(gateKey(n));
                    return (
                      <button
                        key={n}
                        data-gsx-name={n}
                        onClick={() => onPick(n)}
                        title={
                          times === 1
                            ? t("map.gsx.used_once")
                            : times
                              ? t("map.gsx.used_times", { n: String(times) })
                              : undefined
                        }
                        className={`rounded px-1.5 py-0.5 font-mono text-[11px] transition-colors ${
                          picked === n
                            ? "bg-brand-500 text-slate-950"
                            : times
                              ? "bg-brand-500/20 text-brand-100 ring-1 ring-brand-500/40 hover:bg-brand-500/30"
                              : "bg-sf-raised text-sf-text hover:bg-brand-500/20"
                        }`}
                      >
                        {n}
                        {times ? <span className="ml-1 opacity-70">×{times}</span> : null}
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
            {!q && s.missing.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {s.missing.map((m) => (
                  <span
                    key={m.name}
                    title={t("map.gsx.not_in_profile")}
                    className="cursor-default rounded px-1.5 py-0.5 font-mono text-[11px] text-sf-muted border border-dashed border-sf-border"
                  >
                    {m.name}
                    <span className="ml-1 opacity-70">×{m.count}</span>
                  </span>
                ))}
              </div>
            )}
          </div>
        ))}
        {rendered.length === 0 && (
          <p className="py-2 text-center text-[11px] text-sf-muted">{t("map.gsx.no_match")}</p>
        )}
      </div>
    </div>
  );
}

function AirlineChip({
  active,
  title,
  onClick,
  children,
}: {
  active: boolean;
  title?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] transition-colors ${
        active
          ? "border-brand-500/50 bg-brand-500/15 text-brand-100"
          : "border-sf-border/60 bg-sf-surface text-sf-text/90 hover:border-slate-500/60 hover:bg-sf-raised"
      }`}
    >
      {children}
    </button>
  );
}
