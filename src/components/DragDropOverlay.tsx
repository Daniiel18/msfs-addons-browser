import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, CheckCircle2, FileDown, Loader2, Upload } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { InstallResult, InstallerPayload, PtpPayload } from "../lib/types";
import { api, isTauri } from "../lib/tauri";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { useCommunityStore } from "../stores/useCommunityStore";

/**
 * Overlay de Drag & Drop para toda la app.
 *
 * Tauri v2 emite tres eventos del sistema de archivos al arrastrar:
 *   · `tauri://drag-enter`   — el cursor entró con un payload
 *   · `tauri://drag-over`    — el cursor sigue dentro
 *   · `tauri://drag-leave`   — salió sin soltar
 *   · `tauri://drag-drop`    — soltó los archivos
 *
 * Mostramos un overlay translúcido durante enter/over para feedback
 * visual, y procesamos los paths uno por uno cuando llega el drop.
 *
 * Cada archivo se manda a `install_archive`. Tres outcomes por
 * archivo:
 *   1. Paquete MSFS válido → instalado, refresh del scanner.
 *   2. Instalador (.exe / .msi) → notificamos al usuario con la
 *      ruta y un botón para abrir el .exe.
 *   3. Archivo basura → mostramos error.
 *
 * Soportamos las extensiones que `install_archive` ya maneja:
 * .zip, .rar, .7z. Cualquier otra extensión pasa también pero
 * el backend devolverá un error que mostramos al usuario.
 */
export function DragDropOverlay() {
  const [hovering, setHovering] = useState(false);
  const [results, setResults] = useState<DropResult[]>([]);

  const refreshInstalled = useDownloadsStore((s) => s.refreshInstalled);
  const rescanCommunity = useCommunityStore((s) => s.rescan);

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
        const next: DropResult[] = paths.map((p) => ({
          path: p,
          state: "processing" as const,
        }));
        setResults(next);
        for (let i = 0; i < paths.length; i++) {
          const path = paths[i];
          try {
            const res = await invoke<InstallResult>("install_archive", {
              archivePath: path,
              communityPath: null,
            });
            setResults((prev) =>
              prev.map((r, idx) => {
                if (idx !== i) return r;
                // Tres outcomes posibles, en orden de prioridad:
                //   1. PTP livery (PMDG OC) → guardada en inbox.
                //   2. Instalador externo (.exe/.msi) detectado.
                //   3. Paquete MSFS instalado normalmente.
                if (res.ptpPayload) {
                  return { path, state: "ptp", payload: res.ptpPayload };
                }
                if (res.installerPayload) {
                  return {
                    path,
                    state: "installer",
                    payload: res.installerPayload,
                  };
                }
                return {
                  path,
                  state: "installed",
                  packageCount: res.packages.length,
                };
              }),
            );
          } catch (e) {
            setResults((prev) =>
              prev.map((r, idx) =>
                idx === i
                  ? { path, state: "error", error: String(e) }
                  : r,
              ),
            );
          }
        }
        // Reconciliar UI: refresca historial + community scan
        refreshInstalled().catch(() => {});
        rescanCommunity().catch(() => {});
      }),
    );

    return () => {
      Promise.all(unlisteners).then((fns) => fns.forEach((f) => f()));
    };
  }, [refreshInstalled, rescanCommunity]);

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
                Suelta el archivo para instalar
              </p>
              <p className="mt-1 text-xs text-slate-400">
                Soportado: .zip · .rar · .7z (instaladores .exe se detectan)
              </p>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {results.length > 0 && (
        <DropResultsToast results={results} onClear={() => setResults([])} />
      )}
    </>
  );
}

interface DragDropPayload {
  paths?: string[];
}

type DropResult =
  | { path: string; state: "processing" }
  | { path: string; state: "installed"; packageCount: number }
  | { path: string; state: "installer"; payload: InstallerPayload }
  | { path: string; state: "ptp"; payload: PtpPayload }
  | { path: string; state: "error"; error: string };

function DropResultsToast({
  results,
  onClear,
}: {
  results: DropResult[];
  onClear: () => void;
}) {
  const allDone = results.every((r) => r.state !== "processing");
  return (
    <div className="fixed bottom-4 right-4 z-40 w-[min(440px,calc(100vw-2rem))] rounded-xl border border-slate-800 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800 backdrop-blur">
      <header className="flex items-center justify-between border-b border-slate-800 px-4 py-2.5">
        <div className="flex items-center gap-2 text-sm font-medium text-slate-100">
          <FileDown className="h-4 w-4 text-brand-300" />
          {allDone ? "Resultados" : "Procesando archivos…"}
        </div>
        {allDone && (
          <button
            onClick={onClear}
            className="text-xs text-slate-400 hover:text-slate-200"
          >
            Cerrar
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

function DropResultItem({ result }: { result: DropResult }) {
  const fileName = result.path.split(/[\\/]/).pop() ?? result.path;
  return (
    <li className="px-4 py-2.5 text-xs">
      <div className="flex items-start gap-2">
        <div className="mt-0.5 shrink-0">
          {result.state === "processing" && (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-slate-400" />
          )}
          {result.state === "installed" && (
            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />
          )}
          {result.state === "installer" && (
            <FileDown className="h-3.5 w-3.5 text-amber-400" />
          )}
          {result.state === "ptp" && (
            <FileDown className="h-3.5 w-3.5 text-sky-400" />
          )}
          {result.state === "error" && (
            <AlertCircle className="h-3.5 w-3.5 text-rose-400" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-slate-200">{fileName}</div>
          {result.state === "processing" && (
            <p className="text-[11px] text-slate-500">Extrayendo e instalando…</p>
          )}
          {result.state === "installed" && (
            <p className="text-[11px] text-emerald-300">
              Instalado · {result.packageCount} paquete
              {result.packageCount === 1 ? "" : "s"}
            </p>
          )}
          {result.state === "installer" && (
            <InstallerNote payload={result.payload} />
          )}
          {result.state === "ptp" && <PtpNote payload={result.payload} />}
          {result.state === "error" && (
            <p className="text-[11px] text-rose-300">{result.error}</p>
          )}
        </div>
      </div>
    </li>
  );
}

function PtpNote({ payload }: { payload: PtpPayload }) {
  const count = payload.ptpFiles.length;
  return (
    <div className="mt-1">
      <p className="text-[11px] text-sky-300">
        {count === 1
          ? "Livery PMDG (.ptp) guardada en el Inbox"
          : `${count} liveries PMDG (.ptp) guardadas en el Inbox`}
        . Abre <span className="font-medium">PMDG Operations Center</span> →{" "}
        <span className="font-medium">Install Livery</span> y apunta a esta
        carpeta para importarla en el avión correspondiente.
      </p>
      <button
        onClick={() => api.openLocalPath(payload.inboxDir)}
        className="mt-1 text-[11px] text-sky-200 underline hover:text-sky-100"
      >
        Abrir Inbox
      </button>
    </div>
  );
}

function InstallerNote({ payload }: { payload: InstallerPayload }) {
  const exeName = payload.primaryInstaller.split(/[\\/]/).pop() ?? "instalador";
  return (
    <div className="mt-1">
      <p className="text-[11px] text-amber-300">
        Este archivo no es un paquete MSFS — contiene un instalador (
        <span className="font-mono">{exeName}</span>). Te dejamos los archivos
        extraídos en disco para que lo ejecutes manualmente.
      </p>
      <button
        onClick={() => api.openLocalPath(payload.extractedDir)}
        className="mt-1 text-[11px] text-amber-200 underline hover:text-amber-100"
      >
        Abrir carpeta extraída
      </button>
    </div>
  );
}
