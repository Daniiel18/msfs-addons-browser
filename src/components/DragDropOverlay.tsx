import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  FileDown,
  KeyRound,
  Loader2,
  Upload,
} from "lucide-react";
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
  } | null>(null);
  const [fastDeleting, setFastDeleting] = useState(false);
  // (v5.2.0) Path de un archivo cifrado esperando contraseña. `wrong` =
  // el último intento falló por clave incorrecta.
  const [pwPrompt, setPwPrompt] = useState<{ path: string; wrong: boolean } | null>(
    null,
  );
  // (v5.2.1) Acumulador de archivos origen ya instalados, pendientes de
  // ofrecer su borrado. Se vuelca (flushDeletePrompt) solo cuando NO
  // quedan contraseñas pendientes — así el prompt de borrado no aparece
  // a medias ni se solapa con el de contraseña en un drop masivo con
  // varios cifrados.
  const deleteAccumRef = useRef<string[]>([]);

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
      // (v6.2.49) Descarga interceptada del navegador embebido de liveries →
      // la tratamos EXACTAMENTE como un archivo soltado (inspect → modal →
      // install). Así el livery descargado con la cuenta del usuario se instala
      // con el mismo pipeline (nested-zip, detección de variante, etc.).
      listen<string>("livery-download://finished", async (event) => {
        const path = event.payload;
        if (path) await processDropBatch([path]);
      }),
    );

    return () => {
      Promise.all(unlisteners).then((fns) => fns.forEach((f) => f()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const processDropBatch = async (paths: string[], password?: string) => {
    // (v5.2.0) Upsert por path: en el reintento con contraseña, el path
    // ya existe en la lista — lo volvemos a "inspecting" en vez de
    // duplicarlo. Soporta archivos Y carpetas (el backend resuelve).
    setResults((prev) => {
      const next = [...prev];
      for (const p of paths) {
        const i = next.findIndex((r) => r.path === p);
        const entry: DropFlowResult = { path: p, state: "inspecting" };
        if (i === -1) next.push(entry);
        else next[i] = entry;
      }
      return next;
    });

    const inspections = await Promise.all(
      paths.map(async (path) => {
        try {
          const insp = (await api.dropInspect(path, password)) as DropInspection;
          return { path, inspection: insp, error: null as string | null };
        } catch (e) {
          return { path, inspection: null, error: String(e) };
        }
      }),
    );

    // (v5.2.1) Archivos que (todavía) requieren contraseña. En un drop
    // masivo puede haber varios — los recogemos TODOS para reintentarlos
    // juntos con la misma clave (los de un mismo paquete suelen
    // compartirla) en vez de dejar los demás colgados.
    const pwPendingList = inspections.filter(
      (i) =>
        !!i.error &&
        (i.error.includes("NEEDS_PASSWORD") ||
          i.error.includes("WRONG_PASSWORD")),
    );

    setResults((prev) =>
      prev.map((r) => {
        const m = inspections.find((i) => i.path === r.path);
        if (!m || r.state !== "inspecting") return r;
        if (m.error) {
          if (
            m.error.includes("NEEDS_PASSWORD") ||
            m.error.includes("WRONG_PASSWORD")
          ) {
            return { path: r.path, state: "awaiting_password" };
          }
          return { path: r.path, state: "error", error: m.error };
        }
        if (m.inspection && m.inspection.items.length === 0) {
          void api.dropCancel(m.inspection.sessionId).catch(() => {});
          return {
            path: r.path,
            state: "error",
            error: t("drop.no_installable"),
          };
        }
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

    const allSingle =
      validInspections.length > 0 &&
      validInspections.every((i) => i.items.length === 1);

    if (validInspections.length > 0 && allSingle) {
      // FAST-PATH: todo de 1 item (liveries, aviones sueltos…). Commit
      // directo, marca instalado YA y acumula los archivos para ofrecer
      // su borrado al final (cuando no quede ninguna clave pendiente).
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
          // (v5.2.0) NO ofrecemos borrar el original si era una CARPETA
          // (el usuario no espera que se borre su carpeta de origen).
          if (
            !insp.isFolder &&
            (r.installedGsx.length > 0 ||
              r.installedPackages.length > 0 ||
              (r.installedConfigs?.length ?? 0) > 0 ||
              (r.installedLiveries?.length ?? 0) > 0)
          ) {
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
      await onDoneBatch(validInspections, reports);
      deleteAccumRef.current.push(...committed);
    } else if (validInspections.length > 0) {
      // Multi-item → modal de selección paginado.
      setActiveInspections(validInspections);
    }

    // (v5.2.1) Resolución de contraseñas: si aún quedan archivos
    // cifrados, pedimos UNA clave (se reintentarán todos juntos). Si no
    // queda ninguna, recién ahí volcamos el prompt de borrado de los
    // archivos origen — así no aparece a medias ni se solapa con el de
    // contraseña.
    if (pwPendingList.length > 0) {
      setPwPrompt({
        path: pwPendingList[0].path,
        wrong: pwPendingList[0].error!.includes("WRONG_PASSWORD"),
      });
    } else {
      flushDeletePrompt();
    }
  };

  /** (v5.2.1) Vuelca los archivos acumulados al prompt de borrado, si
   *  hay alguno. Se llama cuando ya no quedan contraseñas pendientes. */
  const flushDeletePrompt = () => {
    const archives = Array.from(new Set(deleteAccumRef.current));
    deleteAccumRef.current = [];
    if (archives.length > 0) setFastDelete({ archives });
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
                // (v5.2.1) Los paquetes ya se marcaron "instalado" vía
                // onDoneBatch al hacer commit; aquí solo cerramos.
                setFastDelete(null);
              }}
            />
          </div>
        </div>
      )}

      {/* (v5.2.0) Prompt de contraseña para archivos cifrados. */}
      {pwPrompt && (
        <PasswordModal
          fileName={pwPrompt.path.split(/[\\/]/).pop() ?? pwPrompt.path}
          wrong={pwPrompt.wrong}
          onSubmit={(pw) => {
            const fallback = pwPrompt.path;
            setPwPrompt(null);
            // (v5.2.1) Reintentar TODOS los archivos que esperan
            // contraseña con esta clave — los de un mismo paquete suelen
            // compartirla. Los que sigan fallando volverán a pedir clave.
            const awaiting = results
              .filter((r) => r.state === "awaiting_password")
              .map((r) => r.path);
            void processDropBatch(awaiting.length > 0 ? awaiting : [fallback], pw);
          }}
          onCancel={() => {
            setPwPrompt(null);
            // (v5.2.1) Cancelar = abortar la entrada de claves: marca
            // como cancelados TODOS los que esperaban contraseña (no solo
            // este) para que la app no quede "procesando" indefinidamente,
            // y ofrece borrar los que sí se instalaron.
            setResults((prev) =>
              prev.map((r) =>
                r.state === "awaiting_password"
                  ? { path: r.path, state: "cancelled" }
                  : r,
              ),
            );
            flushDeletePrompt();
          }}
        />
      )}

      {results.length > 0 && (
        <DropResultsToast results={results} onClear={() => setResults([])} />
      )}
    </>
  );
}

/** (v5.2.0) Modal para introducir la contraseña de un archivo cifrado
 *  (.rar/.7z/.zip). Reintenta la inspección con la clave. */
function PasswordModal({
  fileName,
  wrong,
  onSubmit,
  onCancel,
}: {
  fileName: string;
  wrong: boolean;
  onSubmit: (pw: string) => void;
  onCancel: () => void;
}) {
  const [pw, setPw] = useState("");
  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-slate-950/75 p-4 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-amber-300" />
          <h3 className="text-sm font-semibold text-slate-100">
            {t("drop.pw.title")}
          </h3>
        </div>
        <p className="mt-1.5 truncate font-mono text-[11px] text-slate-400" title={fileName}>
          {fileName}
        </p>
        <input
          autoFocus
          type="password"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && pw.trim()) onSubmit(pw);
            else if (e.key === "Escape") onCancel();
          }}
          placeholder={t("drop.pw.placeholder")}
          className="mt-3 w-full rounded-md border border-slate-700 bg-slate-950/60 px-3 py-2 text-sm text-slate-100 placeholder:text-slate-600 focus:border-brand-500/50 focus:outline-none"
        />
        {wrong && (
          <p className="mt-1.5 text-[11px] text-rose-300">{t("drop.pw.wrong")}</p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={() => pw.trim() && onSubmit(pw)}
            disabled={!pw.trim()}
            className="rounded-md bg-brand-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-400 disabled:opacity-50"
          >
            {t("drop.pw.unlock")}
          </button>
        </div>
      </div>
    </div>
  );
}

interface DragDropPayload {
  paths?: string[];
}

type DropFlowResult =
  | { path: string; state: "inspecting" }
  | { path: string; state: "awaiting_password" }
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
    (r) =>
      r.state !== "inspecting" &&
      r.state !== "awaiting_modal" &&
      r.state !== "awaiting_password",
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
          {result.state === "awaiting_password" && (
            <KeyRound className="h-3.5 w-3.5 text-amber-300" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-slate-200">{fileName}</div>
          {result.state === "inspecting" && (
            <p className="text-[11px] text-slate-500">
              {t("drop.extracting")}
            </p>
          )}
          {result.state === "awaiting_password" && (
            <p className="text-[11px] text-amber-300">{t("drop.pw.pending")}</p>
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
  if ((report.installedLiveries?.length ?? 0) > 0) {
    parts.push(
      t("drop.installed.liveries", {
        count: String(report.installedLiveries?.length ?? 0),
      }),
    );
  }
  if ((report.installedConfigs?.length ?? 0) > 0) {
    parts.push(
      t("drop.installed.configs", {
        count: String(report.installedConfigs?.length ?? 0),
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
