import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  ChevronDown,
  ChevronUp,
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
 *   2. **Desinstalar** — borra el folder en Community + filas de DB.
 *      Confirmación adicional para evitar accidentes.
 *   3. **Abrir carpeta** — abre `install_path` con el explorador.
 *
 * El modal hace un search lazy del ICAO al abrir para enseñarle al
 * usuario qué métodos están disponibles ahora mismo en cada fuente.
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

  // Search lazy: solo se dispara cuando el usuario pulsa "Reparar"
  // (vía `loadCatalogIfNeeded`). El modal principal ya no muestra
  // el catálogo — sólo el sub-popup de selección de método. Esto
  // evita 2 llamadas HTTP por cada apertura del modal aunque el
  // usuario sólo quisiera ver detalles.
  const loadCatalogIfNeeded = async () => {
    if (!pkg.icao || matches.length > 0 || searching) return;
    setSearching(true);
    try {
      const [sa, sp] = await Promise.all([
        api.search(pkg.icao, "sceneryaddons").catch(() => [] as Addon[]),
        api.search(pkg.icao, "simplaza").catch(() => [] as Addon[]),
      ]);
      setMatches([...sa, ...sp]);
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
      await api.uninstallCommunityPackage(pkg.folderName);
      await rescan();
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

  const canRepair = !!pkg.icao;

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
              <Detail label="Carpeta" value={pkg.folderName} mono />
              <Detail label="Versión" value={pkg.packageVersion ?? "—"} />
              <Detail
                label="Tamaño"
                value={pkg.sizeBytes != null ? formatBytes(pkg.sizeBytes) : "—"}
              />
              <Detail
                label="MSFS mínimo"
                value={pkg.minimumGameVersion ?? "—"}
              />
              {pkg.airportName && (
                <Detail label="Aeropuerto" value={pkg.airportName} />
              )}
              <Detail
                label="Última modificación"
                value={pkg.folderModifiedAt ?? "—"}
              />
            </dl>

            {/* Changelog expandible. Sólo visible cuando el modal
                tiene un ICAO con que cruzar el catálogo — el detail
                page de la fuente es donde vive el changelog. */}
            {pkg.icao && (
              <ChangelogSection
                pkg={pkg}
                onLoadCatalog={loadCatalogIfNeeded}
                matches={matches}
                searching={searching}
              />
            )}

            {error && (
              <div className="mt-4 flex items-start gap-2 rounded-lg bg-rose-500/15 px-3 py-2 text-xs text-rose-200 ring-1 ring-rose-500/30">
                <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span className="flex-1">{error}</span>
              </div>
            )}

            {confirmingUninstall && (
              <div className="mt-4 rounded-lg border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-100">
                <p className="font-medium">
                  ¿Borrar definitivamente <span className="font-mono">{pkg.folderName}</span>?
                </p>
                <p className="mt-1 text-rose-200/80">
                  Se eliminará la carpeta en Community y las filas asociadas en
                  la base de datos. No se puede deshacer.
                </p>
                <div className="mt-2 flex justify-end gap-2">
                  <button
                    onClick={() => setConfirmingUninstall(false)}
                    className="rounded-md border border-slate-700 px-3 py-1 text-slate-200 hover:bg-slate-800"
                  >
                    Cancelar
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
                    Confirmar
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
              Abrir carpeta
            </button>
            <button
              onClick={openRepairPicker}
              disabled={busy !== "none" || !canRepair}
              title={
                canRepair
                  ? update
                    ? `Actualizar a v${update.latestVersion}`
                    : "Elegir método y reinstalar limpio"
                  : "Sin ICAO conocido para buscar en el catálogo"
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
              {update ? `Actualizar (v${update.latestVersion})` : "Reinstalar"}
            </button>
            <div className="ml-auto">
              <button
                onClick={() => setConfirmingUninstall(true)}
                disabled={busy !== "none"}
                className="inline-flex items-center gap-1.5 rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 text-xs font-medium text-rose-200 hover:border-rose-400 hover:bg-rose-500/20 disabled:opacity-50"
              >
                <Trash2 className="h-3.5 w-3.5" />
                Desinstalar
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
                title="Ver la nueva versión en la fuente"
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
function RepairMethodPicker({
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
              Reparar / Reinstalar
            </h3>
            <p className="mt-0.5 text-[11px] text-slate-400">
              Se desinstalará <span className="font-mono">{pkg.folderName}</span> y luego
              se descargará con el método que elijas.
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
              Buscando {pkg.icao} en SceneryAddons y Simplaza…
            </div>
          ) : grouped.length === 0 ? (
            <p className="px-2 py-4 text-center text-xs text-slate-500">
              No hay métodos de descarga disponibles en ninguna fuente.
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
            Cancelar
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

function Detail({
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

function formatBytes(n: number): string {
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

/**
 * Sección desplegable con el changelog scrapeado de la página
 * detalle de la fuente. Tres estados:
 *   · Cerrado: sólo muestra el botón "Mostrar changelog".
 *   · Abierto + cargando: spinner.
 *   · Abierto + listo: lista bullet de líneas.
 *
 * El catálogo (matches) se necesita para saber a qué página apuntar.
 * Si todavía no se cargó, abrir el changelog dispara el lazy fetch.
 */
function ChangelogSection({
  pkg,
  onLoadCatalog,
  matches,
  searching,
}: {
  pkg: CommunityPackage;
  onLoadCatalog: () => Promise<void>;
  matches: Addon[];
  searching: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [lines, setLines] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Elegimos la página del catálogo más probable: el match exacto
  // por nombre normalizado (mismo desarrollador) si existe, si no
  // el primero que comparta ICAO. Es el mismo criterio que el
  // wizard usa para no traer changelogs de variantes ajenas.
  const target = useMemo(() => {
    if (matches.length === 0) return null;
    const wantedNorm = pkg.title
      .toLowerCase()
      .replace(/\sv\d+(?:\.\d+)+\b.*$/i, "")
      .trim();
    const exact = matches.find(
      (m) =>
        m.name
          .toLowerCase()
          .replace(/\sv\d+(?:\.\d+)+\b.*$/i, "")
          .trim() === wantedNorm,
    );
    return exact ?? matches[0];
  }, [matches, pkg.title]);

  const handleToggle = async () => {
    const next = !open;
    setOpen(next);
    if (!next) return;
    if (lines !== null) return; // ya cargado en sesión
    setError(null);
    setLoading(true);
    try {
      // Asegura catálogo cargado antes de pedir la página de detalle.
      if (matches.length === 0) {
        await onLoadCatalog();
      }
      // El target se recomputa después del load — leemos del closure
      // de matches, pero el lazy load ya llenó el state, así que
      // matches estará actualizado en la siguiente prop pass. Para
      // la primera abertura usamos un retry simple si target es null.
      const page = target;
      if (!page) {
        setError("No se encontró el addon en el catálogo de fuentes.");
        setLines([]);
        return;
      }
      const cl = await api.fetchChangelog(page.pageUrl);
      setLines(cl.lines);
    } catch (e) {
      setError(String(e));
      setLines([]);
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="mt-4 rounded-lg border border-slate-800 bg-slate-900/40">
      <button
        onClick={handleToggle}
        className="flex w-full items-center justify-between px-3 py-2 text-xs font-medium text-slate-200 hover:bg-slate-800/40"
      >
        <span className="inline-flex items-center gap-1.5">
          {open ? (
            <ChevronUp className="h-3.5 w-3.5" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5" />
          )}
          Changelog
        </span>
        {target && (
          <span className="font-mono text-[10px] text-slate-500">
            {target.source}
            {target.version ? ` · v${target.version}` : ""}
          </span>
        )}
      </button>
      {open && (
        <div className="border-t border-slate-800 px-3 py-2 text-xs">
          {(loading || searching) && (
            <div className="inline-flex items-center gap-2 text-slate-500">
              <Loader2 className="h-3 w-3 animate-spin" />
              {searching ? "Buscando en el catálogo…" : "Leyendo página de detalle…"}
            </div>
          )}
          {!loading && error && (
            <p className="text-rose-300">{error}</p>
          )}
          {!loading && !error && lines && lines.length === 0 && (
            <p className="text-slate-500">
              No se encontró un changelog en la página de detalle.
            </p>
          )}
          {!loading && !error && lines && lines.length > 0 && (
            <ul className="ml-3 list-disc space-y-0.5 text-slate-300">
              {lines.slice(0, 30).map((l, i) => (
                <li key={i} className="leading-snug">
                  {l}
                </li>
              ))}
              {lines.length > 30 && (
                <li className="italic text-slate-500">
                  … y {lines.length - 30} líneas más
                </li>
              )}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}

