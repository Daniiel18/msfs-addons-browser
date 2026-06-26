import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { t } from "../lib/i18n";
import {
  AlertCircle,
  Box,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  FileDown,
  Loader2,
  Plane,
  Trash2,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import type { DropCommitReport, DropInspection, DropItem } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";

/**
 * (v2.1.0, refactor v2.2.0) Modal selector multi-archivo.
 *
 * Antes (v2.1.x): modal por archivo. Si el usuario soltaba 3 .rar,
 * había 3 modales secuenciales. Si uno fallaba, los siguientes
 * quedaban orphan en la cola.
 *
 * Ahora (v2.2.0): UN modal con paginación interna. Recibe TODAS las
 * inspecciones de un drop y muestra "Archivo X de N ← →" con
 * checkboxes por archivo. Al pulsar "Install" commitea TODAS las
 * sesiones secuencialmente y reporta el total agregado.
 *
 * (v4.7.0) Tras una instalación exitosa, ofrece BORRAR el/los archivo(s)
 * comprimido(s) original(es) arrastrados (paso de confirmación). Además
 * todos los textos pasan por i18n (`t(...)`) — antes había strings en
 * español hardcoded que se mostraban aunque la app estuviera en inglés.
 */
interface Props {
  inspections: DropInspection[];
  onClose: () => void;
  onDone: (reports: DropCommitReport[]) => void;
}

export function DropSelectModal({ inspections, onClose, onDone }: Props) {
  const [pageIdx, setPageIdx] = useState(0);
  const inspection = inspections[pageIdx];

  // (v2.2.0) Selecciones por sessionId — preservamos el estado al
  // navegar entre páginas. Cada Set tiene las sourcePath checked.
  const [selectionsBySession, setSelectionsBySession] = useState<
    Record<string, Set<string>>
  >(() => {
    const init: Record<string, Set<string>> = {};
    for (const insp of inspections) {
      init[insp.sessionId] = new Set(insp.items.map((it) => it.sourcePath));
    }
    return init;
  });
  const [installing, setInstalling] = useState(false);
  // (v4.11.2) Estado efímero "Instalado ✓" que se muestra ~1s ANTES de
  // ofrecer borrar el archivo — así se percibe que SÍ se instaló y no
  // parece que se borra sin instalar.
  const [installedOk, setInstalledOk] = useState(false);
  // (v4.7.0) Paso de borrado del archivo original tras instalar.
  const [deletePrompt, setDeletePrompt] = useState<string[] | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [pendingReports, setPendingReports] = useState<DropCommitReport[]>([]);

  const selected = selectionsBySession[inspection.sessionId] ?? new Set();

  // ESC cierra (con cancel de TODAS las sesiones).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === "Escape" &&
        !installing &&
        !deleting &&
        !deletePrompt &&
        !installedOk
      ) {
        cancelAll();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installing, deleting, deletePrompt, installedOk]);

  const cancelAll = async () => {
    for (const insp of inspections) {
      await api.dropCancel(insp.sessionId).catch(() => {});
    }
    onClose();
  };

  const toggle = (path: string) =>
    setSelectionsBySession((prev) => {
      const next = { ...prev };
      const current = new Set(next[inspection.sessionId] ?? []);
      if (current.has(path)) current.delete(path);
      else current.add(path);
      next[inspection.sessionId] = current;
      return next;
    });

  const selectAll = () =>
    setSelectionsBySession((prev) => ({
      ...prev,
      [inspection.sessionId]: new Set(
        inspection.items.map((it) => it.sourcePath),
      ),
    }));
  const selectNone = () =>
    setSelectionsBySession((prev) => ({
      ...prev,
      [inspection.sessionId]: new Set(),
    }));

  // Conteo total para el footer.
  const totalSelected = useMemo(() => {
    let n = 0;
    for (const s of Object.values(selectionsBySession)) n += s.size;
    return n;
  }, [selectionsBySession]);

  const onInstall = async () => {
    setInstalling(true);
    const reports: DropCommitReport[] = [];
    const committed: string[] = [];
    try {
      // Commitea cada inspección secuencialmente. Si alguna falla,
      // seguimos con las siguientes — el reporte agregado lo dice.
      for (const insp of inspections) {
        const sel = selectionsBySession[insp.sessionId];
        if (!sel || sel.size === 0) {
          // Skip + cancel para liberar tempdir.
          await api.dropCancel(insp.sessionId).catch(() => {});
          continue;
        }
        try {
          const r = await api.dropCommit(
            insp.sessionId,
            Array.from(sel),
            null,
          );
          reports.push(r);
          // (v4.14.0 #5c) Solo ofrecemos borrar el archivo origen si
          // REALMENTE se instaló algo desde él — perfil GSX o paquete
          // Community. Antes se empujaba `archivePath` aunque el commit
          // no instalara nada (solo errores), lo que ofrecía borrar un
          // archivo que no se había instalado. Con esto, los drops de
          // perfiles GSX (.zip/.rar/.7z → %AppData%\Virtuali\GSX\MSFS)
          // disparan el confirm de borrado igual que los de Community.
          if (
            r.installedGsx.length > 0 ||
            r.installedPackages.length > 0 ||
            (r.installedConfigs?.length ?? 0) > 0
          ) {
            committed.push(insp.archivePath);
          }
        } catch (e) {
          reports.push({
            installedGsx: [],
            installedPackages: [],
            installedConfigs: [],
            errors: [
              `${insp.archivePath.split(/[\\/]/).pop() ?? "archivo"}: ${String(e)}`,
            ],
          });
        }
      }
      void useCommunityStore.getState().rescan().catch(() => {});
      void useGsxLocalStore.getState().refresh().catch(() => {});
    } finally {
      setInstalling(false);
    }

    // (v4.7.0 / v4.11.2 / v5.0.1) Si algo se instaló bien, mostramos
    // "Instalado ✓" ~2s y LUEGO ofrecemos borrar el/los archivo(s). Si
    // no, cerramos.
    const uniqueArchives = Array.from(new Set(committed));
    if (uniqueArchives.length > 0) {
      setPendingReports(reports);
      setInstalledOk(true);
      setTimeout(() => {
        setInstalledOk(false);
        setDeletePrompt(uniqueArchives);
      }, 2000);
    } else {
      onDone(reports);
    }
  };

  // (v4.7.0) Resuelve el paso de borrado: borra (o no) y cierra.
  const resolveDelete = async (doDelete: boolean) => {
    const archives = deletePrompt ?? [];
    if (doDelete && archives.length > 0) {
      setDeleting(true);
      for (const p of archives) {
        await api.deleteDroppedArchive(p).catch(() => {});
      }
      setDeleting(false);
    }
    setDeletePrompt(null);
    onDone(pendingReports);
  };

  const onCancel = async () => {
    if (installing || deleting || deletePrompt) return;
    await cancelAll();
  };

  // Agrupamos por tipo para mostrar secciones.
  const gsxItems = inspection.items.filter((i) => i.kind === "gsx_profile");
  const communityItems = inspection.items.filter(
    (i) => i.kind === "community_package",
  );
  // (v6.1) Configs de avión (iFly/PMDG) — sección propia, NO mezclar con GSX.
  const configItems = inspection.items.filter((i) =>
    ["ifly_config", "pmdg_config"].includes(i.kind),
  );
  const otherItems = inspection.items.filter(
    (i) =>
      ![
        "gsx_profile",
        "community_package",
        "ifly_config",
        "pmdg_config",
      ].includes(i.kind),
  );

  return (
    <AnimatePresence>
      <motion.div
        key="drop-modal"
        className="fixed inset-0 z-[60] flex items-center justify-center overflow-y-auto bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={onCancel}
      >
        <motion.div
          initial={{ y: 16, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 16, opacity: 0 }}
          transition={{ duration: 0.18 }}
          onClick={(e) => e.stopPropagation()}
          className={`w-full rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ${
            installedOk || deletePrompt ? "max-w-sm" : "max-w-2xl"
          }`}
        >
          {installedOk ? (
            <InstalledSuccess reports={pendingReports} />
          ) : deletePrompt ? (
            <DeleteConfirm
              archives={deletePrompt}
              deleting={deleting}
              onResolve={resolveDelete}
            />
          ) : (
            <>
              <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
                <div className="flex items-center gap-2">
                  <FileDown className="h-4 w-4 text-brand-300" />
                  <div>
                    <h2 className="text-sm font-semibold text-slate-100">
                      {t("drop.modal.title")}
                    </h2>
                    <p
                      className="truncate text-[11px] text-slate-500 max-w-[450px]"
                      title={inspection.archivePath}
                    >
                      {inspection.archivePath.split(/[\\/]/).pop()}
                    </p>
                  </div>
                </div>
                <button
                  onClick={onCancel}
                  disabled={installing}
                  className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
                >
                  <X className="h-4 w-4" />
                </button>
              </header>

              {/* (v2.2.0) Paginación cuando hay varios archivos. */}
              {inspections.length > 1 && (
                <div className="flex items-center justify-between gap-2 border-b border-slate-800 bg-slate-900/30 px-5 py-2">
                  <button
                    onClick={() => setPageIdx((i) => Math.max(0, i - 1))}
                    disabled={pageIdx === 0 || installing}
                    className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2 py-1 text-[11px] text-slate-300 hover:border-slate-700 disabled:opacity-30"
                  >
                    <ChevronLeft className="h-3 w-3" />
                    {t("drop.modal.prev")}
                  </button>
                  <div className="text-center text-[11px] text-slate-300">
                    <span className="font-mono">
                      {t("drop.modal.file_of", {
                        n: pageIdx + 1,
                        total: inspections.length,
                      })}
                    </span>
                    <span className="ml-2 text-slate-500">
                      {t("drop.modal.selected_here", {
                        sel: (
                          selectionsBySession[inspection.sessionId] ?? new Set()
                        ).size,
                        total: inspection.items.length,
                      })}
                    </span>
                  </div>
                  <button
                    onClick={() =>
                      setPageIdx((i) =>
                        Math.min(inspections.length - 1, i + 1),
                      )
                    }
                    disabled={pageIdx === inspections.length - 1 || installing}
                    className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2 py-1 text-[11px] text-slate-300 hover:border-slate-700 disabled:opacity-30"
                  >
                    {t("drop.modal.next")}
                    <ChevronRight className="h-3 w-3" />
                  </button>
                </div>
              )}

              <div className="flex items-center justify-between gap-2 border-b border-slate-800 px-5 py-2 text-[11px]">
                <span className="text-slate-500">
                  {t("drop.modal.items_detected", {
                    count: inspection.items.length,
                  })}{" "}
                  ·{" "}
                  <span className="text-emerald-300">
                    {t("drop.modal.selected_count", { count: selected.size })}
                  </span>
                </span>
                <div className="flex gap-1">
                  <button
                    onClick={selectAll}
                    className="rounded border border-slate-800 px-2 py-0.5 text-slate-400 hover:border-slate-700 hover:text-slate-200"
                  >
                    {t("drop.modal.select_all")}
                  </button>
                  <button
                    onClick={selectNone}
                    className="rounded border border-slate-800 px-2 py-0.5 text-slate-400 hover:border-slate-700 hover:text-slate-200"
                  >
                    {t("drop.modal.select_none")}
                  </button>
                </div>
              </div>

              <div className="max-h-[55vh] overflow-y-auto p-4">
                {inspection.items.length === 0 && (
                  <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-xs text-amber-200">
                    <AlertCircle className="mr-1 inline h-3.5 w-3.5" />
                    {t("drop.modal.no_items")}
                  </div>
                )}

                {gsxItems.length > 0 && (
                  <Section
                    title={t("drop.tab.gsx")}
                    icon={<Plane className="h-3 w-3 text-violet-300" />}
                    items={gsxItems}
                    selected={selected}
                    toggle={toggle}
                    destinationHint="%APPDATA%\Virtuali\GSX\MSFS"
                  />
                )}
                {communityItems.length > 0 && (
                  <Section
                    title={t("drop.tab.community")}
                    icon={<Box className="h-3 w-3 text-emerald-300" />}
                    items={communityItems}
                    selected={selected}
                    toggle={toggle}
                    destinationHint="MSFS Community"
                  />
                )}
                {configItems.length > 0 && (
                  <Section
                    title={t("drop.tab.config")}
                    icon={<Plane className="h-3 w-3 text-sky-300" />}
                    items={configItems}
                    selected={selected}
                    toggle={toggle}
                    destinationHint={t("drop.config.dest")}
                  />
                )}
                {otherItems.length > 0 && (
                  <Section
                    title={t("drop.tab.other")}
                    icon={<AlertCircle className="h-3 w-3 text-slate-500" />}
                    items={otherItems}
                    selected={selected}
                    toggle={toggle}
                    destinationHint=""
                  />
                )}
              </div>

              <footer className="flex items-center justify-between gap-2 border-t border-slate-800 bg-slate-900/40 px-5 py-3">
                <span className="text-[11px] text-slate-500">
                  {t("drop.modal.footer_hint")}
                </span>
                <div className="flex gap-2">
                  <button
                    onClick={onCancel}
                    disabled={installing}
                    className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700 disabled:opacity-50"
                  >
                    {t("drop.modal.cancel")}
                  </button>
                  <button
                    onClick={onInstall}
                    disabled={installing || totalSelected === 0}
                    className="inline-flex items-center gap-1 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400 disabled:opacity-40"
                  >
                    {installing ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <CheckCircle2 className="h-3.5 w-3.5" />
                    )}
                    {t("drop.modal.install", {
                      count: totalSelected,
                      items:
                        totalSelected === 1
                          ? t("drop.modal.item")
                          : t("drop.modal.items"),
                    })}
                    {inspections.length > 1 &&
                      t("drop.modal.from_files", { n: inspections.length })}
                  </button>
                </div>
              </footer>
            </>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}

/** (v4.11.2) Estado efímero "Instalado ✓" antes de ofrecer borrar. Da
 *  feedback claro de que la instalación SÍ ocurrió. */
function InstalledSuccess({ reports }: { reports: DropCommitReport[] }) {
  const count = reports.reduce(
    (n, r) =>
      n +
      r.installedPackages.length +
      r.installedGsx.length +
      (r.installedConfigs?.length ?? 0),
    0,
  );
  return (
    <div className="flex flex-col items-center gap-2 px-6 py-8 text-center">
      <motion.div
        initial={{ scale: 0.6, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        transition={{ type: "spring", stiffness: 320, damping: 18 }}
        className="rounded-full bg-emerald-500/15 p-3 ring-1 ring-emerald-500/30"
      >
        <CheckCircle2 className="h-7 w-7 text-emerald-300" />
      </motion.div>
      <h2 className="text-sm font-semibold text-slate-100">
        {t("drop.modal.installed_ok")}
      </h2>
      <p className="text-xs text-slate-400">
        {t("drop.modal.installed_ok_sub", { count })}
      </p>
    </div>
  );
}

/** (v4.7.0) Paso de confirmación: ¿borrar el/los archivo(s) original(es)?
 *  (v4.23.0) Exportado: el fast-path de DragDropOverlay (archivos de 1
 *  solo item, p.ej. liveries) lo reutiliza — antes ese camino instalaba
 *  directo y NUNCA ofrecía borrar el archivo (bug reportado). */
export function DeleteConfirm({
  archives,
  deleting,
  onResolve,
}: {
  archives: string[];
  deleting: boolean;
  onResolve: (doDelete: boolean) => void;
}) {
  return (
    <div className="p-5">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-lg bg-rose-500/15 p-2 ring-1 ring-rose-500/30">
          <Trash2 className="h-5 w-5 text-rose-300" />
        </div>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-slate-100">
            {t("drop.modal.delete_title")}
          </h2>
          <p className="mt-1 text-xs leading-relaxed text-slate-400">
            {t("drop.modal.delete_body")}
          </p>
          <div className="mt-3">
            <p className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-slate-500">
              {t("drop.modal.delete_files")}
            </p>
            <ul className="max-h-32 space-y-1 overflow-y-auto">
              {archives.map((p) => (
                <li
                  key={p}
                  title={p}
                  className="truncate rounded border border-slate-800 bg-slate-900/40 px-2 py-1 font-mono text-[11px] text-slate-300"
                >
                  {p.split(/[\\/]/).pop()}
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-end gap-2">
        <button
          onClick={() => onResolve(false)}
          disabled={deleting}
          className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700 disabled:opacity-50"
        >
          {t("drop.modal.delete_no")}
        </button>
        <button
          onClick={() => onResolve(true)}
          disabled={deleting}
          className="inline-flex items-center gap-1 rounded-md bg-rose-500 px-3 py-1.5 text-xs font-semibold text-rose-950 hover:bg-rose-400 disabled:opacity-50"
        >
          {deleting ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Trash2 className="h-3.5 w-3.5" />
          )}
          {deleting ? t("drop.modal.deleting") : t("drop.modal.delete_yes")}
        </button>
      </div>
    </div>
  );
}

function Section({
  title,
  icon,
  items,
  selected,
  toggle,
  destinationHint,
}: {
  title: string;
  icon: React.ReactNode;
  items: DropItem[];
  selected: Set<string>;
  toggle: (path: string) => void;
  destinationHint: string;
}) {
  return (
    <section className="mb-4 last:mb-0">
      <h3 className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-slate-400">
        {icon}
        {title} ({items.length})
        {destinationHint && (
          <span className="ml-auto font-mono text-[9px] text-slate-600">
            → {destinationHint}
          </span>
        )}
      </h3>
      <ul className="space-y-1">
        {items.map((it) => {
          const displayName = it.sourcePath.split(/[\\/]/).pop() ?? it.relativePath;
          return (
            <li
              key={it.sourcePath}
              onClick={() =>
                !["installer_exe", "unknown"].includes(it.kind) &&
                toggle(it.sourcePath)
              }
              className={`flex cursor-pointer items-start gap-2 rounded-md border px-3 py-2 text-xs transition-colors ${
                selected.has(it.sourcePath)
                  ? "border-emerald-500/40 bg-emerald-500/5"
                  : "border-slate-800 bg-slate-900/40 hover:border-slate-700"
              }`}
            >
              <input
                type="checkbox"
                checked={selected.has(it.sourcePath)}
                disabled={["installer_exe", "unknown"].includes(it.kind)}
                onChange={() => toggle(it.sourcePath)}
                onClick={(e) => e.stopPropagation()}
                className="mt-0.5 accent-emerald-500 disabled:opacity-30"
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  {it.icao && (
                    <span className="font-mono text-[11px] font-semibold text-brand-300">
                      {it.icao}
                    </span>
                  )}
                  <span className="truncate font-medium text-slate-200">
                    {displayName}
                  </span>
                </div>
                {/* (v2.1.1) Variant chips — VDGS / noVDGS / handler / locale */}
                {it.variants.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {it.variants.map((v, i) => (
                      <span
                        key={i}
                        className={`inline-flex items-center rounded px-1.5 py-0.5 text-[9px] uppercase tracking-wide ${variantChipClass(
                          v,
                        )}`}
                      >
                        {v}
                      </span>
                    ))}
                  </div>
                )}
                {/* Subfolder + size */}
                <div className="mt-0.5 truncate font-mono text-[10px] text-slate-500">
                  {it.relativePath} · {formatBytes(it.sizeBytes)}
                </div>
                {/* Description (.ini header preview) */}
                {it.description && (
                  <details className="mt-1 text-[10px] text-slate-400">
                    <summary className="cursor-pointer text-slate-500 hover:text-slate-300">
                      {t("drop.modal.see_first_lines")}
                    </summary>
                    <pre className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap rounded border border-slate-800 bg-slate-950/50 px-2 py-1.5 font-mono text-[10px] leading-relaxed text-slate-400">
                      {it.description}
                    </pre>
                  </details>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** (v2.1.1) Colorea los chips de variantes según su semántica.
 *  Sirve para que el usuario distinga rápidamente VDGS de noVDGS, etc. */
function variantChipClass(variant: string): string {
  const v = variant.toLowerCase();
  if (v.includes("sin vdgs") || v.includes("novdgs")) {
    return "bg-rose-500/15 text-rose-300 ring-1 ring-rose-500/30";
  }
  if (v.includes("con vdgs") || v.includes("vdgs")) {
    return "bg-sky-500/15 text-sky-300 ring-1 ring-sky-500/30";
  }
  if (v.includes("safedock")) {
    return "bg-amber-500/15 text-amber-300 ring-1 ring-amber-500/30";
  }
  if (v.includes("handler") || v.includes("python")) {
    return "bg-violet-500/15 text-violet-300 ring-1 ring-violet-500/30";
  }
  if (v.startsWith("idioma")) {
    return "bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-500/30";
  }
  if (v.startsWith("variant")) {
    return "bg-brand-500/15 text-brand-300 ring-1 ring-brand-500/30";
  }
  return "bg-slate-700/40 text-slate-300 ring-1 ring-slate-700";
}
