import { useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  ChevronDown,
  Folder,
  Gauge,
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
} from "../lib/types";
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
import { PerformanceModal } from "./PerformanceModal";
import { RegionBadge } from "./RegionBadge";
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
}: {
  pkg: CommunityPackage;
  update: AvailableUpdate | null;
  onClose: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [perfOpen, setPerfOpen] = useState(false);
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
      className="absolute right-3 top-3 z-20 flex max-h-[calc(100%-1.5rem)] w-[330px] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800 backdrop-blur"
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
          <div className="flex h-full w-full items-center justify-center text-slate-700">
            <MapPin className="h-10 w-10" />
          </div>
        )}
        <button
          onClick={onClose}
          disabled={busy !== "none"}
          className="absolute right-2 top-2 rounded-md bg-slate-950/70 p-1 text-slate-300 backdrop-blur hover:bg-slate-900 hover:text-slate-100 disabled:opacity-50"
        >
          <X className="h-3.5 w-3.5" />
        </button>
        <div className="pointer-events-none absolute bottom-2 left-2 flex flex-wrap gap-1">
          {pkg.icao && (
            <span className="rounded bg-emerald-500/90 px-1.5 py-0.5 font-mono text-[10px] font-bold text-emerald-950 shadow">
              {pkg.icao}
            </span>
          )}
          {pkg.icao && <RegionBadge icao={pkg.icao} />}
          {pkg.contentType && (
            <span className="rounded bg-slate-950/80 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-slate-300 ring-1 ring-slate-700">
              {pkg.contentType}
            </span>
          )}
          {update && (
            <span className="rounded bg-amber-500/90 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wide text-amber-950 shadow">
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
            className="truncate text-sm font-semibold text-slate-100"
            title={pkg.airportName ?? pkg.title}
          >
            {pkg.airportName ?? pkg.title}
          </h3>
          <p className="mt-0.5 truncate text-[11px] text-slate-500">
            {pkg.creator ?? "—"}
            {pkg.packageVersion && ` · v${pkg.packageVersion}`}
          </p>
        </div>

        {/* (v5.0.0) Acceso al Modal de Rendimiento (FPS) — desactiva
            autos/pasajeros/GSE/clutter opcionales para ganar FPS. */}
        <div className="px-4 pb-1 pt-1">
          <button
            onClick={() => setPerfOpen(true)}
            className="flex w-full items-center justify-between rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs font-semibold text-emerald-200 transition-colors hover:border-emerald-400/50 hover:bg-emerald-500/20"
          >
            <span className="inline-flex items-center gap-1.5">
              <Gauge className="h-3.5 w-3.5" />
              {t("perf.open_button")}
            </span>
            <ChevronDown className="h-3.5 w-3.5 -rotate-90 text-emerald-300/70" />
          </button>
        </div>

        {/* Acordeón "Ficha técnica" — las filas del viejo modal. */}
        <div className="px-4 pb-3 pt-2">
          <button
            onClick={() => setExpanded((v) => !v)}
            className="flex w-full items-center justify-between rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-2 text-xs font-medium text-slate-200 transition-colors hover:border-brand-500/40 hover:bg-slate-900"
          >
            {t("map.card.tech_info")}
            <ChevronDown
              className={`h-3.5 w-3.5 text-slate-400 transition-transform ${
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
                className="rounded-md border border-slate-700 px-3 py-1 text-slate-200 hover:bg-slate-800"
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
      <footer className="flex shrink-0 items-center gap-1.5 border-t border-slate-800 px-3 py-2.5">
        <button
          onClick={() => void api.openLocalPath(pkg.installPath)}
          disabled={busy !== "none"}
          className="inline-flex items-center gap-1 rounded-lg border border-slate-700 bg-slate-800/60 px-2.5 py-1.5 text-[11px] font-medium text-slate-200 hover:border-brand-500/40 hover:bg-slate-800 disabled:opacity-50"
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

      {/* (v5.0.0) Modal de Rendimiento (FPS). */}
      <AnimatePresence>
        {perfOpen && (
          <PerformanceModal pkg={pkg} onClose={() => setPerfOpen(false)} />
        )}
      </AnimatePresence>
    </motion.div>
  );
}
