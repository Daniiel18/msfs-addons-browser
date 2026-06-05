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

  const selected = selectionsBySession[inspection.sessionId] ?? new Set();

  // ESC cierra (con cancel de TODAS las sesiones).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !installing) {
        cancelAll();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installing]);

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
        } catch (e) {
          reports.push({
            installedGsx: [],
            installedPackages: [],
            errors: [
              `${insp.archivePath.split(/[\\/]/).pop() ?? "archivo"}: ${String(e)}`,
            ],
          });
        }
      }
      onDone(reports);
      void useCommunityStore.getState().rescan().catch(() => {});
      void useGsxLocalStore.getState().refresh().catch(() => {});
    } finally {
      setInstalling(false);
    }
  };

  const onCancel = async () => {
    if (installing) return;
    await cancelAll();
  };

  // Agrupamos por tipo para mostrar secciones.
  const gsxItems = inspection.items.filter((i) => i.kind === "gsx_profile");
  const communityItems = inspection.items.filter(
    (i) => i.kind === "community_package",
  );
  const otherItems = inspection.items.filter(
    (i) => !["gsx_profile", "community_package"].includes(i.kind),
  );

  return (
    <AnimatePresence>
      <motion.div
        key="drop-modal"
        className="fixed inset-0 z-[60] flex items-start justify-center overflow-y-auto bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
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
          className="w-full max-w-2xl rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl"
        >
          <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
            <div className="flex items-center gap-2">
              <FileDown className="h-4 w-4 text-brand-300" />
              <div>
                <h2 className="text-sm font-semibold text-slate-100">
                  ¿Qué quieres instalar?
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
                Anterior
              </button>
              <div className="text-center text-[11px] text-slate-300">
                <span className="font-mono">
                  Archivo {pageIdx + 1} de {inspections.length}
                </span>
                <span className="ml-2 text-slate-500">
                  ({(selectionsBySession[inspection.sessionId] ?? new Set()).size}/
                  {inspection.items.length} seleccionados aquí)
                </span>
              </div>
              <button
                onClick={() =>
                  setPageIdx((i) => Math.min(inspections.length - 1, i + 1))
                }
                disabled={pageIdx === inspections.length - 1 || installing}
                className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2 py-1 text-[11px] text-slate-300 hover:border-slate-700 disabled:opacity-30"
              >
                Siguiente
                <ChevronRight className="h-3 w-3" />
              </button>
            </div>
          )}

          <div className="flex items-center justify-between gap-2 border-b border-slate-800 px-5 py-2 text-[11px]">
            <span className="text-slate-500">
              {inspection.items.length} items detectados ·{" "}
              <span className="text-emerald-300">
                {selected.size} seleccionados
              </span>
            </span>
            <div className="flex gap-1">
              <button
                onClick={selectAll}
                className="rounded border border-slate-800 px-2 py-0.5 text-slate-400 hover:border-slate-700 hover:text-slate-200"
              >
                Todos
              </button>
              <button
                onClick={selectNone}
                className="rounded border border-slate-800 px-2 py-0.5 text-slate-400 hover:border-slate-700 hover:text-slate-200"
              >
                Ninguno
              </button>
            </div>
          </div>

          <div className="max-h-[55vh] overflow-y-auto p-4">
            {inspection.items.length === 0 && (
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-xs text-amber-200">
                <AlertCircle className="mr-1 inline h-3.5 w-3.5" />
                No detectamos perfiles GSX ni paquetes Community en el
                archivo. Verifica que el .zip/.rar contenga .ini, .py o
                una carpeta con manifest.json.
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
            {otherItems.length > 0 && (
              <Section
                title={t("drop.tab.other")}
                icon={
                  <AlertCircle className="h-3 w-3 text-slate-500" />
                }
                items={otherItems}
                selected={selected}
                toggle={toggle}
                destinationHint=""
              />
            )}
          </div>

          <footer className="flex items-center justify-between gap-2 border-t border-slate-800 bg-slate-900/40 px-5 py-3">
            <span className="text-[11px] text-slate-500">
              GSX → %APPDATA%\Virtuali\GSX\MSFS · Community → tu carpeta
              Community detectada
            </span>
            <div className="flex gap-2">
              <button
                onClick={onCancel}
                disabled={installing}
                className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700 disabled:opacity-50"
              >
                Cancelar
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
                Instalar {totalSelected} item{totalSelected === 1 ? "" : "s"}
                {inspections.length > 1 && ` (de ${inspections.length} archivos)`}
              </button>
            </div>
          </footer>
        </motion.div>
      </motion.div>
    </AnimatePresence>
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
                      Ver primeras líneas del archivo
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
