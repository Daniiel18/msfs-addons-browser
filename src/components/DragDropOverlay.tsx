import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, CheckCircle2, FileDown, Loader2, Upload } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import type { DropCommitReport, DropInspection } from "../lib/types";
import { api, isTauri } from "../lib/tauri";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { DropSelectModal, DeleteConfirm } from "./DropSelectModal";
import { t } from "../lib/i18n";

/**
 * (v2.2.0) Overlay de Drag & Drop universal con modal multi-archivo.
 *
 * Flujo:
 *   1. Usuario suelta N archivos.
 *   2. La app inspecciona TODOS en paralelo (extrae cada .zip/.rar
 *      a su tempdir, clasifica los items).
 *   3. Si SOLO hay items en 1 archivo → no abre modal, instala directo.
 *   4. Si hay items en >1 archivos → abre 1 modal con paginación
 *      interna ("Archivo 1 de 3 ← →"). El usuario navega + selecciona.
 *   5. Al "Instalar" → commitea todas las sesiones secuencialmente.
 *   6. Cleanup automático de tempdirs al final.
 *
 * Cambio respecto a v2.1.x: ya no procesamos un archivo a la vez con
 * un modal por archivo. Recolectamos las inspecciones primero y
 * mostramos UN solo modal paginado.
 */
export function DragDropOverlay() {
  const [hovering, setHovering] = useState(false);
  const [activeInspections, setActiveInspections] = useState<
    DropInspection[] | null
  >(null);
  const [results, setResults] = useState<DropFlowResult[]>([]);
  // (v4.23.0) Confirmación de borrado del archivo origen para el
  // FAST-PATH (archivos de 1 solo item, p.ej. liveries): antes ese
  // camino instalaba directo y nunca preguntaba si borrar el archivo —
  // el modal solo salía con GSX/escenarios multi-item (bug reportado).
  const [fastDelete, setFastDelete] = useState<{
    archives: string[];
    inspections: DropInspection[];
    reports: DropCommitReport[];
  } | null>(null);
  const [fastDeleting, setFastDeleting] = useState(false);

  const refreshInstalled = useDownloadsStore((s) => s.refreshInstalled);
  const rescanCommunity = useCommunityStore((s) => s.rescan);
  const refreshGsx = useGsxLocalStore((s) => s.refresh);

  useEffect(() => {
    if (!isTauri) return;
    const unlisteners: Array<Promise<() => void>> = [];

    unlisteners.push(
      listen("tauri://drag-enter", () => setHovering(true)),
      listen("tauri://drag-over", () => setHovering(true)),
      listen("tauri://drag-leave", () => setHovering(false)),
      listen<DragDropPayload>("tauri://drag-drop", async (event) => {
        setHovering(false);
        const paths = event.payload?.paths ?? [];
        if (paths.length === 0) return;
        await processDropBatch(paths);
      }),
    );

    return () => {
      Promise.all(unlisteners).then((fns) => fns.forEach((f) => f()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const processDropBatch = async (paths: string[]) => {
    // Inspeccionamos TODOS los archivos en paralelo. Para cada uno,
    // creamos un result inicial "inspecting". Acumulamos las
    // inspecciones válidas (con items > 0) para el modal.
    const pendingResults: DropFlowResult[] = paths.map((p) => ({
      path: p,
      state: "inspecting" as const,
    }));
    setResults((prev) => [...prev, ...pendingResults]);

    const inspections = await Promise.all(
      paths.map(async (path) => {
        try {
          const insp = await api.dropInspect(path);
          return { path, inspection: insp as DropInspection, error: null };
        } catch (e) {
          return { path, inspection: null, error: String(e) };
        }
      }),
    );

    // Actualizar results según éxito de inspección.
    setResults((prev) =>
      prev.map((r) => {
        const m = inspections.find((i) => i.path === r.path);
        if (!m || r.state !== "inspecting") return r;
        if (m.error) {
          return { path: r.path, state: "error", error: m.error };
        }
        if (m.inspection && m.inspection.items.length === 0) {
          // No instalable.
          void api.dropCancel(m.inspection.sessionId).catch(() => {});
          return {
            path: r.path,
            state: "error",
            error: t("drop.no_installable"),
          };
        }
        // Inspeccionado, esperando decisión del modal.
        return {
          path: r.path,
          state: "awaiting_modal",
          items: m.inspection?.items.length ?? 0,
        };
      }),
    );

    const validInspections = inspections
      .map((i) => i.inspection)
      .filter((i): i is DropInspection => !!i && i.items.length > 0);

    if (validInspections.length === 0) {
      return; // Todos fallaron o estaban vacíos.
    }

    // Si todos los archivos tienen exactamente 1 item, saltamos el
    // modal e instalamos directo todo en lote.
    const allSingle =
      validInspections.length > 0 &&
      validInspections.every((i) => i.items.length === 1);
    if (allSingle) {
      const reports: DropCommitReport[] = [];
      const committed: string[] = [];
      for (const insp of validInspections) {
        try {
          const r = await api.dropCommit(
            insp.sessionId,
            [insp.items[0].sourcePath],
            null,
          );
          reports.push(r);
          // (v4.23.0) Igual que el modal: si se instaló algo desde este
          // archivo, ofrecemos borrarlo — TODO lo que entre por drag &
          // drop pregunta, no solo GSX/escenarios.
          if (r.installedGsx.length > 0 || r.installedPackages.length > 0) {
            committed.push(insp.archivePath);
          }
        } catch (e) {
          reports.push({
            installedGsx: [],
            installedPackages: [],
            errors: [String(e)],
          });
        }
      }
      const uniqueArchives = Array.from(new Set(committed));
      if (uniqueArchives.length > 0) {
        setFastDelete({
          archives: uniqueArchives,
          inspections: validInspections,
          reports,
        });
        return;
      }
      onDoneBatch(validInspections, reports);
      return;
    }

    // Hay al menos 1 archivo con varios items — abrir modal paginado.
    setActiveInspections(validInspections);
  };

  const refreshUis = async () => {
    refreshInstalled().catch(() => {});
    rescanCommunity().catch(() => {});
    refreshGsx().catch(() => {});
  };

  const onDoneBatch = async (
    inspections: DropInspection[],
    reports: DropCommitReport[],
  ) => {
    // Actualiza el result toaster por archivo.
    setResults((prev) =>
      prev.map((r) => {
        if (r.state !== "awaiting_modal") return r;
        const idx = inspections.findIndex((i) => i.archivePath === r.path);
        if (idx === -1) return r;
        const report = reports[idx] ?? {
          installedGsx: [],
          installedPackages: [],
          errors: [],
        };
        return { path: r.path, state: "installed", report };
      }),
    );
    setActiveInspections(null);
    await refreshUis();
  };

  const onModalClose = () => {
    if (!activeInspections) return;
    setResults((prev) =>
      prev.map((r) =>
        r.state === "awaiting_modal" &&
        activeInspections.some((i) => i.archivePath === r.path)
          ? { path: r.path, state: "cancelled" }
          : r,
      ),
    );
    setActiveInspections(null);
  };

  return (
    <>
      <AnimatePresence>
        {hovering && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.12 }}
            className="pointer-events-none fixed inset-0 z-40 flex items-center justify-center bg-brand-500/10 backdrop-blur-sm"
          >
            <div className="rounded-2xl border-2 border-dashed border-brand-400 bg-slate-950/80 px-8 py-10 text-center shadow-2xl">
              <Upload className="mx-auto h-12 w-12 text-brand-300" />
              <p className="mt-3 text-base font-semibold text-slate-100">
                {t("drop.drop_to_install")}
              </p>
              <p className="mt-1 text-xs text-slate-400">
                {t("drop.formats_hint")}
              </p>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {activeInspections && (
        <DropSelectModal
          inspections={activeInspections}
          onClose={onModalClose}
          onDone={(reports) => onDoneBatch(activeInspections, reports)}
        />
      )}

      {/* (v4.23.0) Confirmación de borrado del archivo origen para el
          fast-path (liveries y cualquier drop de 1 item). */}
      {fastDelete && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80 px-4 backdrop-blur-sm">
          <div className="w-full max-w-md overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950 shadow-2xl ring-1 ring-slate-800/70">
            <DeleteConfirm
              archives={fastDelete.archives}
              deleting={fastDeleting}
              onResolve={async (doDelete) => {
                if (doDelete) {
                  setFastDeleting(true);
                  for (const p of fastDelete.archives) {
                    await api.deleteDroppedArchive(p).catch(() => {});
                  }
                  setFastDeleting(false);
                }
                const fd = fastDelete;
                setFastDelete(null);
                void onDoneBatch(fd.inspections, fd.reports);
              }}
            />
          </div>
        </div>
      )}

      {results.length > 0 && (
        <DropResultsToast results={results} onClear={() => setResults([])} />
      )}
    </>
  );
}

interface DragDropPayload {
  paths?: string[];
}

type DropFlowResult =
  | { path: string; state: "inspecting" }
  | { path: string; state: "awaiting_modal"; items: number }
  | { path: string; state: "installed"; report: DropCommitReport }
  | { path: string; state: "cancelled" }
  | { path: string; state: "error"; error: string };

function DropResultsToast({
  results,
  onClear,
}: {
  results: DropFlowResult[];
  onClear: () => void;
}) {
  const allDone = results.every(
    (r) => r.state !== "inspecting" && r.state !== "awaiting_modal",
  );
  return (
    <div className="fixed bottom-4 right-4 z-40 w-[min(440px,calc(100vw-2rem))] rounded-xl border border-slate-800 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800 backdrop-blur">
      <header className="flex items-center justify-between border-b border-slate-800 px-4 py-2.5">
        <div className="flex items-center gap-2 text-sm font-medium text-slate-100">
          <FileDown className="h-4 w-4 text-brand-300" />
          {allDone ? t("drop.results") : t("drop.processing")}
        </div>
        {allDone && (
          <button
            onClick={onClear}
            className="text-xs text-slate-400 hover:text-slate-200"
          >
            {t("common.close")}
          </button>
        )}
      </header>
      <ul className="max-h-72 overflow-y-auto divide-y divide-slate-800">
        {results.map((r, i) => (
          <DropResultItem key={i} result={r} />
        ))}
      </ul>
    </div>
  );
}

function DropResultItem({ result }: { result: DropFlowResult }) {
  const fileName = result.path.split(/[\\/]/).pop() ?? result.path;
  return (
    <li className="px-4 py-2.5 text-xs">
      <div className="flex items-start gap-2">
        <div className="mt-0.5 shrink-0">
          {(result.state === "inspecting" ||
            result.state === "awaiting_modal") && (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-slate-400" />
          )}
          {result.state === "installed" && (
            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />
          )}
          {result.state === "error" && (
            <AlertCircle className="h-3.5 w-3.5 text-rose-400" />
          )}
          {result.state === "cancelled" && (
            <AlertCircle className="h-3.5 w-3.5 text-slate-500" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-slate-200">{fileName}</div>
          {result.state === "inspecting" && (
            <p className="text-[11px] text-slate-500">
              {t("drop.extracting")}
            </p>
          )}
          {result.state === "awaiting_modal" && (
            <p className="text-[11px] text-amber-300">
              {t("drop.items_detected", { count: String(result.items) })}
            </p>
          )}
          {result.state === "installed" && <Installed report={result.report} />}
          {result.state === "error" && (
            <p className="text-[11px] text-rose-300">{result.error}</p>
          )}
          {result.state === "cancelled" && (
            <p className="text-[11px] text-slate-500">{t("drop.cancelled")}</p>
          )}
        </div>
      </div>
    </li>
  );
}

function Installed({ report }: { report: DropCommitReport }) {
  const parts: string[] = [];
  if (report.installedGsx.length > 0) {
    parts.push(
      t("drop.installed.gsx", { count: String(report.installedGsx.length) }),
    );
  }
  if (report.installedPackages.length > 0) {
    parts.push(
      t("drop.installed.packages", {
        count: String(report.installedPackages.length),
      }),
    );
  }
  if (parts.length === 0) parts.push(t("drop.installed.nothing"));
  return (
    <p className="text-[11px] text-emerald-300">
      {t("drop.installed.prefix")} · {parts.join(" + ")}
      {report.errors.length > 0 && (
        <span className="ml-1 text-rose-300">
          · {report.errors.length} {t("common.errors")}
        </span>
      )}
    </p>
  );
}
