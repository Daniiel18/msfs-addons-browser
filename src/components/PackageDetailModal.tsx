import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  Download,
  ExternalLink,
  Folder,
  Loader2,
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
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { useToastStore } from "../stores/useToastStore";
import { derivedType, diagnosePackage } from "../lib/packageType";
import { t } from "../lib/i18n";

interface Props {
  pkg: CommunityPackage;
  update: AvailableUpdate | null;
  onClose: () => void;
}

/**
 * Modal con el detalle de un paquete instalado en Community.
 *
 * Tres acciones principales en la fila inferior:
 *   1. **Reparar** — abre un sub-popup con todos los métodos de
 *      descarga disponibles (en cada fuente). El usuario elige uno
 *      → desinstalamos + descargamos. Esto sustituye el "Reparar"
 *      original que elegía silenciosamente el primer torrent y
 *      sorprendía al usuario.
 *   2. **Desinstalar** — borra el folder en TODAS las Community
 *      detectadas (Steam/MSStore × 2020/2024) + carpetas extras +
 *      filas de DB. Confirmación adicional para evitar accidentes;
 *      el resultado se reporta vía toast (paths borrados / fallidos).
 *   3. **Abrir carpeta** — abre `install_path` con el explorador.
 *
 * El modal hace un search lazy del ICAO al abrir para enseñarle al
 * usuario qué métodos están disponibles ahora mismo en cada fuente.
 *
 * Cambio v0.1.19: la sección **Changelog** se eliminó de este
 * modal — vivía como acordeón para paquetes con ICAO (escenarios)
 * pero el usuario reportó que era ruido en el detalle de un paquete
 * ya instalado. El changelog ahora vive en un botón dentro de los
 * cards de búsqueda de Simplaza (`ResultCard.tsx`).
 */
export function PackageDetailModal({ pkg, update, onClose }: Props) {
  const [busy, setBusy] = useState<"none" | "uninstalling" | "repairing">("none");
  const [confirmingUninstall, setConfirmingUninstall] = useState(false);
  const [pickingRepair, setPickingRepair] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [matches, setMatches] = useState<Addon[]>([]);
  const [searching, setSearching] = useState(false);

  const rescan = useCommunityStore((s) => s.rescan);
  const startDownload = useDownloadsStore((s) => s.start);
  const pushToast = useToastStore((s) => s.push);

  // Buscamos por ICAO si lo tenemos (SCENERY) y, además, por
  // nombre/keyword en Simplaza siempre — ese catálogo busca por
  // texto libre y casi todos los addons no-aeropuerto (aviones,
  // liveries, mods) viven allí. Sin esto, "Reinstalar" salía
  // deshabilitado para todo lo que no fuera escenario.
  const searchKeyword = useMemo(() => {
    // Primer término significativo del título — quita prefijos
    // genéricos tipo "Airbus", "Boeing", etc, y se queda con la
    // primera palabra >= 3 letras que aparezca.
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
      // SceneryAddons sólo funciona con ICAO.
      if (pkg.icao) {
        queries.push(api.search(pkg.icao, "sceneryaddons").catch(() => [] as Addon[]));
      }
      // Simplaza acepta queries libres — buscamos por ICAO si hay,
      // y SIEMPRE también por keyword del título. Eso dobla las
      // chances de encontrar el paquete (aviones, liveries, mods).
      if (pkg.icao) {
        queries.push(api.search(pkg.icao, "simplaza").catch(() => [] as Addon[]));
      }
      if (searchKeyword) {
        queries.push(api.search(searchKeyword, "simplaza").catch(() => [] as Addon[]));
      }
      const results = await Promise.all(queries);
      // Dedup por id — la búsqueda por ICAO y por keyword pueden
      // devolver el mismo addon si el título lo incluye.
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

  // ESC cierra cualquier overlay activo de adentro hacia fuera:
  // confirmación → sub-modal → modal principal.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || busy !== "none") return;
      if (confirmingUninstall) {
        setConfirmingUninstall(false);
      } else if (pickingRepair) {
        setPickingRepair(false);
      } else {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, confirmingUninstall, pickingRepair, onClose]);

  const doUninstall = async () => {
    setBusy("uninstalling");
    setError(null);
    try {
      const report = await api.uninstallCommunityPackage(pkg.folderName);
      await rescan();
      // Reporte al usuario — toast verde con cuántas ubicaciones se
      // limpiaron. Si hubo fallos parciales (típicamente MSFS abierto
      // bloqueando handles), un segundo toast rojo los detalla.
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

  /** Reparación con método elegido por el usuario en el sub-popup.
   *  Desinstala el paquete actual y dispara la descarga con el
   *  método seleccionado. El paquete reaparecerá en
   *  `community_packages` después del rescan que el DownloadsStore
   *  hace al completar la instalación. */
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

  const openFolder = () => api.openLocalPath(pkg.installPath);

  /** Abre el sub-popup. Si todavía no tenemos catálogo cacheado en
   *  state, dispara la búsqueda en background — el popup muestra
   *  spinner mientras llega. Mejor UX que bloquear el botón
   *  "Reparar" hasta que las dos fuentes terminen su HTTP. */
  const openRepairPicker = () => {
    setPickingRepair(true);
    void loadCatalogIfNeeded();
  };

  // Reinstalación habilitada si hay CUALQUIER término por el que
  // buscar — ICAO (SCENERY) o keyword del título (todo lo demás).
  // Antes exigíamos `!!pkg.icao` y por eso liveries/aviones quedaban
  // bloqueados. Ahora caen en el camino Simplaza-by-name.
  const canRepair = !!pkg.icao || !!searchKeyword;

  return (
    <AnimatePresence>
      <motion.div
        key="backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={() => busy === "none" && !pickingRepair && onClose()}
        className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 backdrop-blur-sm"
      >
        <motion.div
          key="modal"
          initial={{ opacity: 0, scale: 0.96, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          className="relative w-[min(640px,calc(100vw-2rem))] max-h-[calc(100vh-3rem)] overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-4">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                {pkg.icao && (
                  <span className="rounded bg-emerald-500/15 px-2 py-0.5 font-mono text-xs font-semibold text-emerald-300 ring-1 ring-emerald-500/30">
                    {pkg.icao}
                  </span>
                )}
                {pkg.contentType && (
                  <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-slate-400">
                    {pkg.contentType}
                  </span>
                )}
                {update && (
                  <span className="inline-flex items-center gap-1 rounded bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-300 ring-1 ring-amber-500/30">
                    Update {update.installedVersion} → {update.latestVersion}
                  </span>
                )}
              </div>
              <h2 className="mt-2 truncate text-base font-semibold text-slate-100">
                {pkg.title}
              </h2>
              {pkg.creator && (
                <p className="mt-0.5 text-xs text-slate-400">
                  {pkg.creator}
                  {pkg.packageVersion && ` · v${pkg.packageVersion}`}
                </p>
              )}
            </div>
            <button
              onClick={onClose}
              disabled={busy !== "none"}
              className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="overflow-y-auto px-5 py-4">
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
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

            {/* (v4.28.0) Diagnóstico del clasificador — el usuario pidió
                ver POR QUÉ se etiquetó así y, si es livery, a qué avión
                base apunta. Útil cuando un addon sale mal categorizado:
                muestra la pista exacta (manifest, base_container, etc.)
                para reportar el caso. */}
            <PackageDiagnostic pkg={pkg} />

            {error && (
              <div className="mt-4 flex items-start gap-2 rounded-lg bg-rose-500/15 px-3 py-2 text-xs text-rose-200 ring-1 ring-rose-500/30">
                <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span className="flex-1">{error}</span>
              </div>
            )}

            {confirmingUninstall && (
              <div className="mt-4 rounded-lg border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-100">
                <p className="font-medium">
                  {t("pkg.uninstall_confirm.q")} <span className="font-mono">{pkg.folderName}</span>?
                </p>
                <p className="mt-1 text-rose-200/80">
                  {t("pkg.uninstall_confirm.body")}
                </p>
                <div className="mt-2 flex justify-end gap-2">
                  <button
                    onClick={() => setConfirmingUninstall(false)}
                    className="rounded-md border border-slate-700 px-3 py-1 text-slate-200 hover:bg-slate-800"
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    onClick={doUninstall}
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

          <footer className="flex flex-wrap items-center gap-2 border-t border-slate-800 px-5 py-3">
            <button
              onClick={openFolder}
              disabled={busy !== "none"}
              className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-800/60 px-3 py-1.5 text-xs font-medium text-slate-200 hover:border-brand-500/40 hover:bg-slate-800 disabled:opacity-50"
            >
              <Folder className="h-3.5 w-3.5" />
              {t("pkg.open_folder")}
            </button>
            <button
              onClick={openRepairPicker}
              disabled={busy !== "none" || !canRepair}
              title={
                canRepair
                  ? update
                    ? t("pkg.update_to", { v: update.latestVersion })
                    : pkg.icao
                      ? t("pkg.reinstall_picker")
                      : t("pkg.reinstall_by_name")
                  : t("pkg.cannot_reinstall")
              }
              className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
                update
                  ? "border-amber-400 bg-amber-500 text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
                  : "border-amber-500/40 bg-amber-500/10 text-amber-200 hover:border-amber-400 hover:bg-amber-500/20"
              }`}
            >
              {busy === "repairing" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCcw className="h-3.5 w-3.5" />
              )}
              {update ? t("pkg.update_btn", { v: update.latestVersion }) : t("pkg.reinstall")}
            </button>
            <div className="ml-auto">
              <button
                onClick={() => setConfirmingUninstall(true)}
                disabled={busy !== "none"}
                className="inline-flex items-center gap-1.5 rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 text-xs font-medium text-rose-200 hover:border-rose-400 hover:bg-rose-500/20 disabled:opacity-50"
              >
                <Trash2 className="h-3.5 w-3.5" />
                {t("pkg.uninstall")}
              </button>
            </div>
            {update && (
              <a
                href={update.pageUrl}
                onClick={(e) => {
                  e.preventDefault();
                  api.openExternal(update.pageUrl);
                }}
                className="inline-flex items-center gap-1 text-[11px] text-slate-400 hover:text-slate-200"
                title={t("pkg.view_source")}
              >
                <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </footer>

          {/* Sub-popup de selección de método para Reparar. Vive
              dentro del modal (overlay sobre la card existente)
              para que el usuario no pierda contexto del paquete
              que está reparando. */}
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
      </motion.div>
    </AnimatePresence>
  );
}

/**
 * Sub-popup superpuesto al modal principal con la lista exhaustiva
 * de métodos de descarga disponibles en cada fuente. El usuario
 * elige el que prefiera (torrent recomendado para auto-instalación;
 * mirrors abren el navegador). Cancela vuelve al modal sin tocar
 * nada.
 */
export function RepairMethodPicker({
  pkg,
  matches,
  searching,
  onPick,
  onCancel,
}: {
  pkg: CommunityPackage;
  matches: Addon[];
  searching: boolean;
  onPick: (addon: Addon, method: DownloadMethod) => void;
  onCancel: () => void;
}) {
  // Ordena: torrent primero (mejor para reparar), luego mirror,
  // luego direct. Dentro de cada fuente ordena addons por versión
  // descendente para que el más nuevo aparezca arriba.
  const grouped = useMemo(() => groupBySourceWithMethods(matches), [matches]);

  return (
    <motion.div
      key="repair-backdrop"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      onClick={onCancel}
      className="absolute inset-0 z-10 flex items-center justify-center bg-slate-950/80 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 8 }}
        transition={{ duration: 0.14 }}
        onClick={(e) => e.stopPropagation()}
        className="w-[min(560px,calc(100%-2rem))] max-h-[calc(100%-2rem)] overflow-hidden rounded-xl border border-amber-500/30 bg-slate-950 shadow-2xl ring-1 ring-amber-500/20"
      >
        <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-amber-200">
              {t("pkg.repair.title")}
            </h3>
            <p className="mt-0.5 text-[11px] text-slate-400">
              {t("pkg.repair.body.before")}{" "}
              <span className="font-mono">{pkg.folderName}</span>{" "}
              {t("pkg.repair.body.after")}
            </p>
          </div>
          <button
            onClick={onCancel}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="max-h-[60vh] overflow-y-auto p-4">
          {searching && grouped.length === 0 ? (
            <div className="flex items-center justify-center gap-2 px-2 py-6 text-xs text-slate-400">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("pkg.repair.searching", { icao: pkg.icao ?? "" })}
            </div>
          ) : grouped.length === 0 ? (
            <p className="px-2 py-4 text-center text-xs text-slate-500">
              {t("pkg.repair.no_methods")}
            </p>
          ) : (
            <ul className="space-y-3">
              {grouped.map((g, gi) => (
                <li key={`${g.source}-${gi}`}>
                  <div className="mb-1.5 flex items-center gap-2 text-[11px] text-slate-400">
                    <span className="rounded bg-slate-800 px-1.5 py-0.5 font-semibold uppercase tracking-wide">
                      {g.source}
                    </span>
                    {g.version && <span className="text-slate-500">v{g.version}</span>}
                  </div>
                  <p className="mb-2 truncate text-xs text-slate-200">{g.title}</p>
                  <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                    {g.methods.map((mm, mi) => (
                      <button
                        key={mi}
                        onClick={() => onPick(g.addon, mm)}
                        className={`inline-flex items-center gap-2 rounded-md border px-3 py-2 text-left text-xs font-medium transition-colors ${
                          mm.kind === "torrent"
                            ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-200 hover:border-emerald-400 hover:bg-emerald-500/20"
                            : "border-slate-700 bg-slate-800/60 text-slate-200 hover:border-brand-500/40 hover:bg-slate-800"
                        }`}
                      >
                        <Download className="h-3.5 w-3.5 shrink-0" />
                        <span className="min-w-0 flex-1 truncate">{mm.name}</span>
                        <span className="shrink-0 rounded bg-slate-900/60 px-1.5 py-0.5 text-[10px] uppercase">
                          {mm.kind}
                        </span>
                      </button>
                    ))}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>

        <footer className="flex justify-end gap-2 border-t border-slate-800 px-5 py-3">
          <button
            onClick={onCancel}
            className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-200 hover:bg-slate-800"
          >
            {t("common.cancel")}
          </button>
        </footer>
      </motion.div>
    </motion.div>
  );
}

interface Grouped {
  source: string;
  title: string;
  version: string | null;
  addon: Addon;
  methods: DownloadMethod[];
}

function groupBySourceWithMethods(matches: Addon[]): Grouped[] {
  // Para cada source, quedarnos con un único addon (el de mayor
  // versión) y combinar todos sus métodos. Esto evita duplicar
  // botones cuando hay variaciones del mismo addon en sceneryaddons.
  const order = (k: DownloadMethod["kind"]) =>
    k === "torrent" ? 0 : k === "mirror" ? 1 : 2;
  const bySource = new Map<string, Addon>();
  for (const a of matches) {
    if (a.downloadMethods.length === 0) continue;
    const prev = bySource.get(a.source);
    if (!prev) {
      bySource.set(a.source, a);
      continue;
    }
    if ((a.version ?? "") > (prev.version ?? "")) {
      bySource.set(a.source, a);
    }
  }
  return Array.from(bySource.values()).map((a) => ({
    source: a.source,
    title: a.name,
    version: a.version,
    addon: a,
    methods: [...a.downloadMethods].sort((x, y) => order(x.kind) - order(y.kind)),
  }));
}

export function Detail({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <>
      <dt className="text-slate-500">{label}</dt>
      <dd
        className={`truncate text-slate-200 ${mono ? "font-mono text-[11px]" : ""}`}
        title={value}
      >
        {value}
      </dd>
    </>
  );
}

/** (v1.1.4) Renderiza la fila "Perfil GSX" con badge instalado/no
 *  instalado. Lee del store local hidratado al boot. */
export function GsxProfileRow({ icao }: { icao: string | null }) {
  const installedIcaos = useGsxLocalStore((s) => s.installedIcaos);
  if (!icao) return null;
  const has = installedIcaos.has(icao.toUpperCase());
  return (
    <>
      <dt className="text-slate-500">{t("pkg.gsx_profile")}</dt>
      <dd className="truncate">
        {has ? (
          <span className="inline-flex items-center gap-1 rounded-md bg-violet-500/15 px-1.5 py-0.5 text-[11px] font-medium text-violet-200 ring-1 ring-violet-500/40">
            ✓ {t("pkg.gsx_installed")}
          </span>
        ) : (
          <span className="text-[11px] text-slate-500">{t("pkg.gsx_not_detected")}</span>
        )}
      </dd>
    </>
  );
}

/** (v4.28.0) Bloque de diagnóstico: tipo detectado + razón de ser
 *  livery (si aplica) + contenedor base. Refleja exactamente lo que
 *  decide `derivedType` y `diagnosePackage`. */
function PackageDiagnostic({ pkg }: { pkg: CommunityPackage }) {
  const kind = derivedType(pkg);
  const diag = diagnosePackage(pkg);
  const kindLabel: Record<string, string> = {
    AIRCRAFT: t("pkg.diag.kind.aircraft"),
    LIVERY: t("pkg.diag.kind.livery"),
    INSTRUMENT: t("pkg.diag.kind.utility"),
    MISC: t("pkg.diag.kind.misc"),
    SCENERY: t("pkg.diag.kind.scenery"),
    UNKNOWN: t("pkg.diag.kind.unknown"),
  };
  return (
    <div className="mt-4 rounded-lg border border-slate-800 bg-slate-900/40 p-3 text-xs">
      <div className="mb-1 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
        {t("pkg.diag.title")}
      </div>
      <div className="space-y-1.5">
        <Row label={t("pkg.diag.detected_kind")} value={kindLabel[kind] ?? kind} />
        <Row
          label={t("pkg.diag.is_livery")}
          value={
            diag.isLivery
              ? `${t("common.yes")} — ${diag.liveryReason}`
              : t("common.no")
          }
        />
        {diag.baseContainer && (
          <Row
            label={t("pkg.diag.base_container")}
            value={diag.baseContainer}
            mono
          />
        )}
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="grid grid-cols-[140px_1fr] gap-x-3">
      <div className="text-slate-500">{label}</div>
      <div className={`text-slate-200 ${mono ? "font-mono text-[11px]" : ""}`}>
        {value}
      </div>
    </div>
  );
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const k = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < k.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${k[i]}`;
}
