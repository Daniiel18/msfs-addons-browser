import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { KeyRound, Upload } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { DropCommitReport, DropInspection } from "../lib/types";
import { api, isTauri } from "../lib/tauri";
import { playLandingSound } from "../lib/landingSound";
import { useDownloadsStore, downloadMetaForWindow } from "../stores/useDownloadsStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { useToastStore } from "../stores/useToastStore";
import { DropSelectModal, DeleteConfirm, isEmbeddedDownload } from "./DropSelectModal";
import { ExternalSavedDialog } from "./ExternalSavedDialog";
import { t } from "../lib/i18n";
import { useInstallQueueStore } from "../stores/useInstallQueueStore";
import { formatDropError, isExternalKind, saveExternalItem } from "../lib/externalSave";
import { nativeNotify } from "../lib/nativeNotify";
import {
  discardInternalDownload,
  hasJobKey,
  jobStatus,
  rememberJobKey,
} from "../lib/downloadJobs";

/** (v8.3) Nombre del addon + notas de una descarga del catálogo. */
type DownloadMeta = { displayName?: string; notes: string | null };

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
/** (v6.2.57) Sonido de "toque de pista" + traer la app al frente. Se llama
 *  cuando la instalación/modal de una descarga del embebido está LISTA. */
async function notifyDownloadReady() {
  playLandingSound();
  try {
    const win = getCurrentWindow();
    await win.unminimize().catch(() => {});
    await win.show().catch(() => {});
    await win.setFocus().catch(() => {});
  } catch {
    /* ignore */
  }
}

/** (v6.2.74) Ruteo 2020/2024 de una descarga del embebido según la
 *  compatibilidad que reporta flightsim.to (msfs2020/msfs2024). */
export type DropRouting = {
  communityPath: string | null;
  note: { level: "info" | "warn"; text: string } | null;
  forceModal: boolean;
};

async function computeRoutingFor(insp: DropInspection): Promise<DropRouting> {
  const none: DropRouting = { communityPath: null, note: null, forceModal: false };
  if (!isEmbeddedDownload(insp.archivePath)) return none;
  try {
    const [meta, targets] = await Promise.all([
      api.flightsimDownloadMeta(insp.archivePath),
      api.communityTargets(),
    ]);
    if (!meta) return none;
    const only2020 = meta.msfs2020 === true && meta.msfs2024 !== true;
    const only2024 = meta.msfs2024 === true && meta.msfs2020 !== true;
    // Compatible con ambos, o sin flags → comportamiento normal (versión actual
    // + oferta de cross-link como hoy).
    if (!only2020 && !only2024) return none;
    if (only2020) {
      if (targets.folder2020) {
        const mismatch = targets.currentIs2024;
        return {
          communityPath: targets.folder2020,
          note: mismatch ? { level: "info", text: t("drop.compat.route_2020") } : null,
          forceModal: mismatch,
        };
      }
      return {
        communityPath: null,
        note: { level: "warn", text: t("drop.compat.missing_2020") },
        forceModal: true,
      };
    }
    // only2024
    if (targets.folder2024) {
      const mismatch = !targets.currentIs2024;
      return {
        communityPath: targets.folder2024,
        note: mismatch ? { level: "info", text: t("drop.compat.route_2024") } : null,
        forceModal: mismatch,
      };
    }
    return {
      communityPath: null,
      note: { level: "warn", text: t("drop.compat.missing_2024") },
      forceModal: true,
    };
  } catch {
    return none;
  }
}

export function DragDropOverlay() {
  const [hovering, setHovering] = useState(false);
  // (Ajustes 19/10) El lote que muestra el modal viene de la COLA de
  // instalación: un lote nuevo ya no reemplaza al abierto (se encola) y un
  // clic fuera lo aparca en Notificaciones en vez de cancelarlo.
  const activeBatch = useInstallQueueStore((s) => s.active);
  const activeInspections = activeBatch?.inspections ?? null;
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
  // (v8.3) Nombre/notas de la descarga por ruta del archivo — sobreviven al
  // reintento con contraseña (que vuelve a llamar processDropBatch).
  const metaByPathRef = useRef(new Map<string, DownloadMeta>());

  // (v8.3) Avisos UNIFICADOS: cada archivo se cuenta con los avisos normales
  // (los de "Descarga iniciada"), no con un panel propio. "Extrayendo…" queda
  // fijo mientras dura; al terminar se cambia por "Instalado" o el error. Los
  // estados con su propia ventana (modal, contraseña, pendiente) no avisan.
  const extractingToastRef = useRef(new Map<string, string>());
  const lastStateRef = useRef(new Map<string, DropFlowResult["state"]>());
  useEffect(() => {
    const toast = useToastStore.getState();
    const seen = new Set<string>();
    const done: string[] = [];
    for (const r of results) {
      seen.add(r.path);
      const prev = lastStateRef.current.get(r.path);
      lastStateRef.current.set(r.path, r.state);
      if (prev === r.state) continue;
      const file = r.path.split(/[\\/]/).pop() ?? r.path;
      const extracting = extractingToastRef.current.get(r.path);
      if (extracting && r.state !== "inspecting") {
        toast.dismiss(extracting);
        extractingToastRef.current.delete(r.path);
      }
      if (r.state === "inspecting" && !extracting) {
        const id = toast.push({ kind: "info", title: t("drop.extracting"), message: file, ttlMs: null });
        extractingToastRef.current.set(r.path, id);
      } else if (r.state === "installed") {
        const errors = r.report.errors.length;
        toast.push({
          kind: errors > 0 ? "error" : "success",
          title: t("drop.installed.prefix"),
          message: `${file} · ${installedSummary(r.report)}${errors > 0 ? ` · ${errors} ${t("common.errors")}` : ""}`,
          ttlMs: errors > 0 ? null : 6000,
        });
        done.push(r.path);
      } else if (r.state === "error") {
        toast.push({ kind: "error", title: t("drop.error_title"), message: `${file}: ${r.error}` });
        done.push(r.path);
      } else if (r.state === "cancelled") {
        done.push(r.path);
      }
    }
    // Filas que ya no están (guardadas en carpeta, etc.): sin aviso colgado.
    for (const [p, id] of extractingToastRef.current) {
      if (!seen.has(p)) {
        toast.dismiss(id);
        extractingToastRef.current.delete(p);
      }
    }
    for (const p of lastStateRef.current.keys()) {
      if (!seen.has(p)) lastStateRef.current.delete(p);
    }
    // Ya avisadas: fuera de la lista (no hay panel que las muestre).
    if (done.length > 0) {
      setResults((prev) => prev.filter((r) => !done.includes(r.path)));
    }
  }, [results]);

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
      listen<{ path?: string; job?: string | null } | string>("livery-download://finished", async (event) => {
        const payload = event.payload;
        const path = typeof payload === "string" ? payload : payload?.path;
        if (!path) return;
        // (v8.3) Job del panel de Descargas → ahí se ve el procesado.
        const job = typeof payload === "string" ? null : payload?.job;
        if (job) rememberJobKey(path, job);
        // (v6.2.57) El sonido + traer-al-frente ya NO van aquí (al empezar a
        // procesar), sino cuando el MODAL está LISTO — se dispara dentro de
        // processDropBatch justo antes de mostrarlo / instalar.
        await processDropBatch([path]);
      }),
      // (v7) Descarga capturada del navegador de HOSTERS (modsfire, mixdrop,
      // get.php de sceneryaddons). Mismo pipeline que las liveries: inspect →
      // modal/instalación → auto-borrado del temporal (`simfleet-mirror-dl`).
      // El payload trae {path, window}: instalamos y LUEGO cerramos la ventana
      // del hoster (cuando el modal ya está listo, no antes).
      listen<
        { path?: string; window?: string; password?: string | null } | string
      >(
        "embedded-download://finished",
        async (event) => {
          const payload = event.payload;
          const path = typeof payload === "string" ? payload : payload?.path;
          const win = typeof payload === "string" ? undefined : payload?.window;
          // (v7) La clave la manda el backend (Skybound la lee de la ficha) →
          // la pasamos a la extracción para que NO pida contraseña.
          const password =
            typeof payload === "string" ? undefined : payload?.password ?? undefined;
          if (!path) return;
          // (v8.3) La ventana es `hoster-dl-<job>` → nombre del addon y notas.
          const meta = win ? downloadMetaForWindow(win) : undefined;
          if (meta) metaByPathRef.current.set(path, meta);
          if (win) rememberJobKey(path, win);
          await processDropBatch([path], password);
          if (win) {
            try {
              const w = await WebviewWindow.getByLabel(win);
              await w?.close();
            } catch {
              /* la ventana ya no existe / cierre manual */
            }
          }
        },
      ),
      // (v7) La descarga ARRANCÓ en el navegador embebido (liveries de
      // flightsim.to, mirrors, modsfire, rapidgator…) → toast de SimFleet para
      // que el usuario sepa que sí inició (antes no había feedback ninguno).
      // (v8.3) La ventana de descarga se oculta al empezar: el progreso sigue en
      // el panel de Descargas (clic en el aviso para abrirlo).
      listen<string>("embedded-download://started", () => {
        useToastStore.getState().push({
          kind: "success",
          title: t("toast.download.started"),
          message: t("toast.download.in_panel"),
          onClick: () => useDownloadsStore.getState().openPanelAt("active"),
        });
      }),
    );

    return () => {
      Promise.all(unlisteners).then((fns) => fns.forEach((f) => f()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // (v8.3) "Listo" + aviso de Windows si SimFleet no está al frente: antes, si
  // te ibas a otra cosa, no te enterabas de que ya se había procesado.
  const announceReady = (insps: DropInspection[]) => {
    for (const insp of insps) jobStatus(insp.archivePath, "ready");
    const names = insps.map(
      (i) => i.displayName ?? i.archivePath.split(/[\\/]/).pop() ?? "",
    );
    if (names.length > 0 && insps.some((i) => hasJobKey(i.archivePath))) {
      void nativeNotify({ title: t("downloads.ready_title"), body: names.join(", ") });
    }
  };

  const processDropBatch = async (paths: string[], password?: string) => {
    for (const p of paths) jobStatus(p, "processing");
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
    for (const i of inspections) {
      if (i.error && !pwPendingList.includes(i)) {
        void discardInternalDownload(i.path, "failed", formatDropError(i.error));
      } else if (i.inspection && i.inspection.items.length === 0) {
        void discardInternalDownload(i.path, "failed", t("drop.no_installable"));
      } else if (pwPendingList.includes(i)) {
        jobStatus(i.path, "ready");
      }
    }

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
          return { path: r.path, state: "error", error: formatDropError(m.error) };
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

    let validInspections = inspections
      .map((i) => i.inspection)
      .filter((i): i is DropInspection => !!i && i.items.length > 0);
    // (v8.3) Nombre del addon y notas de las descargas del catálogo.
    for (const insp of validInspections) {
      const meta = metaByPathRef.current.get(insp.archivePath);
      if (meta) {
        insp.displayName = meta.displayName;
        insp.notes = meta.notes;
      }
    }

    // (v6.2.57) ¿El lote viene del navegador embebido? Sólo entonces sonamos +
    // traemos la app al frente — y JUSTO cuando el modal/instalación está listo.
    const fromEmbedded = validInspections.some((i) =>
      isEmbeddedDownload(i.archivePath),
    );
    let readyNotified = false;
    const notifyReadyOnce = () => {
      if (!fromEmbedded || readyNotified) return;
      readyNotified = true;
      void notifyDownloadReady();
    };

    // (v8.3) Nada va a MSFS (instalador, app, navdata…) → directo al diálogo
    // de carpeta, sin modal: se guarda ahí, se abre el Explorador y se avisa
    // (con las notas si las hay). Si cierra el diálogo o falla, pasa al modal
    // de siempre (y de ahí a Notificaciones) para no perder la descarga.
    const directExternal = validInspections.filter(
      (i) => i.items.length === 1 && i.items[0].kind === "external_content",
    );
    if (directExternal.length > 0) {
      notifyReadyOnce();
      announceReady(directExternal);
      const handled = new Set<string>();
      for (const insp of directExternal) {
        try {
          const saved = await saveExternalItem(insp, insp.items[0]);
          if (!saved) continue;
          handled.add(insp.sessionId);
          jobStatus(insp.archivePath, "saved");
          await api.dropCancel(insp.sessionId).catch(() => {});
          if (isEmbeddedDownload(insp.archivePath)) {
            await api.deleteDroppedArchive(insp.archivePath).catch(() => {});
          }
          metaByPathRef.current.delete(insp.archivePath);
          // (v8.3) Sin fila en "Resultados": parecía "instalado" y no se
          // instaló nada. El aviso de "Guardado" ya lo cuenta.
          setResults((prev) => prev.filter((r) => r.path !== insp.archivePath));
        } catch (e) {
          useToastStore.getState().push({
            kind: "error",
            title: t("external.err"),
            message: formatDropError(String(e)),
          });
        }
      }
      validInspections = validInspections.filter((i) => !handled.has(i.sessionId));
    }

    // (v6.2.74) Ruteo 2020/2024: por cada descarga del embebido decidimos en qué
    // Community va según su compatibilidad. Si hay desajuste de versión (o falta
    // la versión requerida), forzamos el modal para AVISAR.
    const routingMap = new Map<string, DropRouting>();
    for (const insp of validInspections) {
      routingMap.set(insp.sessionId, await computeRoutingFor(insp));
    }
    const anyForceModal = [...routingMap.values()].some((r) => r.forceModal);

    const allSingle =
      validInspections.length > 0 &&
      validInspections.every(
        (i) => i.items.length === 1 && !isExternalKind(i.items[0].kind),
      );

    if (validInspections.length > 0 && allSingle && !anyForceModal && !fromEmbedded) {
      // FAST-PATH: todo de 1 item (liveries, aviones sueltos…). Commit
      // directo, marca instalado YA y acumula los archivos para ofrecer
      // su borrado al final (cuando no quede ninguna clave pendiente).
      // (v7) Las descargas del navegador embebido SIEMPRE pasan por el modal
      // (aunque sean 1 item) — el usuario pidió ver qué se instala antes de
      // instalarlo, incluso para escenarios/addons de un solo paquete.
      const reports: DropCommitReport[] = [];
      const committed: string[] = [];
      for (const insp of validInspections) {
        try {
          const r = await api.dropCommit(
            insp.sessionId,
            [insp.items[0].sourcePath],
            routingMap.get(insp.sessionId)?.communityPath ?? null,
          );
          reports.push(r);
          const installedSomething =
            r.installedGsx.length > 0 ||
            r.installedPackages.length > 0 ||
            (r.installedConfigs?.length ?? 0) > 0 ||
            (r.installedLiveries?.length ?? 0) > 0;
          // (v6.2.56) Descarga del navegador embebido (carpeta temporal
          // `simfleet-livery-dl`): la BORRAMOS sola tras instalar, sin
          // preguntar (el usuario no sabe ni le importa dónde quedó).
          if (isEmbeddedDownload(insp.archivePath) && installedSomething) {
            await api.deleteDroppedArchive(insp.archivePath).catch(() => {});
          } else if (!insp.isFolder && installedSomething) {
            // Archivo que TÚ arrastraste → sí ofrecemos borrar (confirm).
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
      // (v6.2.57) Instalación lista (fast-path) → sonido + app al frente.
      notifyReadyOnce();
      await onDoneBatch(validInspections, reports);
      deleteAccumRef.current.push(...committed);
    } else if (validInspections.length > 0) {
      // Multi-item → modal de selección paginado. Sonamos + traemos al frente
      // AHORA que el modal está listo (no al empezar a procesar).
      notifyReadyOnce();
      announceReady(validInspections.filter((i) => !directExternal.includes(i)));
      useInstallQueueStore.getState().enqueue({
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        inspections: validInspections,
        routing: routingMap,
        createdAt: Date.now(),
      });
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
    for (const insp of inspections) {
      const idx = inspections.indexOf(insp);
      const r = reports[idx];
      if (r && (r.installedGsx.length > 0 || r.installedPackages.length > 0 ||
        (r.installedConfigs?.length ?? 0) > 0 || (r.installedLiveries?.length ?? 0) > 0)) {
        jobStatus(insp.archivePath, "installed");
      }
    }
    // Actualiza el result toaster por archivo.
    setResults((prev) =>
      prev.map((r) => {
        if (r.state !== "awaiting_modal" && r.state !== "pending") return r;
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
    await refreshUis();
  };

  const onModalClose = () => {
    if (!activeInspections) return;
    // (v8.3) Cancelar la instalación de una descarga INTERNA borra su archivo
    // temporal ya (antes quedaba hasta la purga del siguiente arranque) y deja
    // su job "cancelado". Lo arrastrado por el usuario no se toca.
    for (const insp of activeInspections) {
      void discardInternalDownload(insp.archivePath, "cancelled");
    }
    setResults((prev) =>
      prev.map((r) =>
        (r.state === "awaiting_modal" || r.state === "pending") &&
        activeInspections.some((i) => i.archivePath === r.path)
          ? { path: r.path, state: "cancelled" }
          : r,
      ),
    );
    useInstallQueueStore.getState().finishActive();
  };

  // (Ajustes 19/10) Clic fuera / Escape / X → la instalación queda PENDIENTE
  // en Notificaciones (Instalar / Cancelar) en vez de perderse.
  const onModalDefer = () => {
    if (!activeInspections) return;
    setResults((prev) =>
      prev.map((r) =>
        r.state === "awaiting_modal" &&
        activeInspections.some((i) => i.archivePath === r.path)
          ? { path: r.path, state: "pending" }
          : r,
      ),
    );
    useInstallQueueStore.getState().deferActive();
    useToastStore.getState().push({
      kind: "success",
      title: t("installq.deferred.title"),
      message: t("installq.deferred.body"),
    });
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
              <p className="mt-3 text-base font-semibold text-sf-text">
                {t("drop.drop_to_install")}
              </p>
              <p className="mt-1 text-xs text-sf-muted">
                {t("drop.formats_hint")}
              </p>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {activeBatch && activeInspections && (
        <DropSelectModal
          /* (v7) key por-lote: si el set de inspecciones cambia (encoge), el
             modal se REMONTA fresco reseteando pageIdx y re-inicializando las
             selecciones — si no, un pageIdx viejo apuntaría a un índice que ya
             no existe y `inspections[pageIdx].sessionId` reventaría (TypeError). */
          key={activeBatch.id}
          inspections={activeInspections}
          routing={activeBatch.routing}
          onClose={onModalClose}
          onDefer={onModalDefer}
          onSaved={(archivePath) => {
            jobStatus(archivePath, "saved");
            setResults((prev) => prev.filter((r) => r.path !== archivePath));
          }}
          onInstalled={(reports) => void onDoneBatch(activeInspections, reports)}
          onDone={(reports) => {
            void onDoneBatch(activeInspections, reports);
            useInstallQueueStore.getState().finishActive();
          }}
        />
      )}

      {/* (v8.3) "Guardado" con las notas de instalación del contenido externo. */}
      <ExternalSavedDialog />

      {/* (v4.23.0) Confirmación de borrado del archivo origen para el
          fast-path (liveries y cualquier drop de 1 item). */}
      {fastDelete && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80 px-4 backdrop-blur-sm">
          <div className="w-full max-w-md overflow-hidden rounded-2xl border border-sf-border/80 bg-sf-bg shadow-2xl ring-1 ring-sf-border/70">
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
            for (const r of results) {
              if (r.state === "awaiting_password") {
                void discardInternalDownload(r.path, "cancelled");
              }
            }
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
        className="w-full max-w-sm rounded-xl border border-sf-border bg-sf-surface p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-amber-300" />
          <h3 className="text-sm font-semibold text-sf-text">
            {t("drop.pw.title")}
          </h3>
        </div>
        <p className="mt-1.5 truncate font-mono text-[11px] text-sf-muted" title={fileName}>
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
          className="mt-3 w-full rounded-md border border-sf-border bg-sf-bg/60 px-3 py-2 text-sm text-sf-text placeholder:text-sf-muted/70 focus:border-brand-500/50 focus:outline-none"
        />
        {wrong && (
          <p className="mt-1.5 text-[11px] text-rose-300">{t("drop.pw.wrong")}</p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-md border border-sf-border px-3 py-1.5 text-xs text-sf-text/90 hover:bg-sf-raised"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={() => pw.trim() && onSubmit(pw)}
            disabled={!pw.trim()}
            className="rounded-md bg-brand-500 px-3 py-1.5 text-xs font-medium text-slate-950 hover:bg-brand-400 disabled:opacity-50"
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
  // (Ajustes 19/10) Aparcada en Notificaciones (clic fuera del modal).
  | { path: string; state: "pending" }
  | { path: string; state: "error"; error: string };

/** (v8.3) Texto del aviso "Instalado": qué se instaló de un archivo. */
function installedSummary(report: DropCommitReport): string {
  const parts: string[] = [];
  if (report.installedGsx.length > 0) {
    parts.push(t("drop.installed.gsx", { count: String(report.installedGsx.length) }));
  }
  if (report.installedPackages.length > 0) {
    parts.push(t("drop.installed.packages", { count: String(report.installedPackages.length) }));
  }
  if ((report.installedLiveries?.length ?? 0) > 0) {
    parts.push(t("drop.installed.liveries", { count: String(report.installedLiveries?.length ?? 0) }));
  }
  if ((report.installedConfigs?.length ?? 0) > 0) {
    parts.push(t("drop.installed.configs", { count: String(report.installedConfigs?.length ?? 0) }));
  }
  if (parts.length === 0) parts.push(t("drop.installed.nothing"));
  return parts.join(" + ");
}
