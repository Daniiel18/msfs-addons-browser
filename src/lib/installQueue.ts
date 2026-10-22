import { api } from "./tauri";
import { t } from "./i18n";
import type { DropCommitReport } from "./types";
import { isEmbeddedDownload } from "../components/DropSelectModal";
import { useInstallQueueStore, batchLabel, type InstallBatch } from "../stores/useInstallQueueStore";
import { useToastStore } from "../stores/useToastStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { isExternalKind } from "./externalSave";
import { discardInternalDownload, jobStatus } from "./downloadJobs";

/**
 * (Ajustes 19/10) Instala un lote pendiente SIN abrir el modal (botón
 * "Instalar" de la campana). Mismo criterio que el modal con su selección
 * por defecto: todos los elementos. Lo que no va a MSFS (apps, instaladores…)
 * necesita elegir carpeta, así que si el lote los trae se abre el modal en su lugar.
 */
export async function installPendingNow(id: string): Promise<void> {
  const store = useInstallQueueStore.getState();
  const batch = store.pending.find((b) => b.id === id);
  if (!batch) return;
  const needsModal = batch.inspections.some((i) =>
    i.items.some((it) => isExternalKind(it.kind)),
  );
  if (needsModal) {
    store.openPending(id);
    return;
  }
  store.takePending(id);
  const toast = useToastStore.getState();
  const reports: DropCommitReport[] = [];
  for (const insp of batch.inspections) {
    try {
      const r = await api.dropCommit(
        insp.sessionId,
        insp.items.map((it) => it.sourcePath),
        batch.routing.get(insp.sessionId)?.communityPath ?? null,
      );
      reports.push(r);
      const installedSomething =
        r.installedGsx.length > 0 ||
        r.installedPackages.length > 0 ||
        (r.installedConfigs?.length ?? 0) > 0 ||
        (r.installedLiveries?.length ?? 0) > 0;
      // Igual que el modal: la descarga temporal del navegador embebido se
      // borra sola; un archivo que arrastró el usuario NO se toca (aquí no
      // hay paso de confirmación).
      if (installedSomething && isEmbeddedDownload(insp.archivePath)) {
        await api.deleteDroppedArchive(insp.archivePath).catch(() => {});
      }
      // (v8.3) El job del panel de Descargas pasa a "instalado".
      if (installedSomething) jobStatus(insp.archivePath, "installed");
    } catch (e) {
      reports.push({ installedGsx: [], installedPackages: [], errors: [String(e)] });
    }
  }
  const errors = reports.flatMap((r) => r.errors ?? []);
  toast.push(
    errors.length > 0
      ? { kind: "error", title: t("installq.done_errors"), message: errors[0] }
      : { kind: "success", title: t("installq.done"), message: batchLabel(batch) },
  );
  void useCommunityStore.getState().rescan().catch(() => {});
  void useGsxLocalStore.getState().refresh().catch(() => {});
  void useDownloadsStore.getState().refreshInstalled().catch(() => {});
}

/** (Ajustes 19/10) Cancela un lote pendiente y libera sus sesiones. */
export async function cancelPending(id: string): Promise<void> {
  const batch = useInstallQueueStore.getState().takePending(id);
  if (!batch) return;
  await cancelBatchSessions(batch);
}

export async function cancelBatchSessions(batch: InstallBatch): Promise<void> {
  for (const insp of batch.inspections) {
    await api.dropCancel(insp.sessionId).catch(() => {});
    // (v8.3) Descarga interna cancelada desde Notificaciones: su archivo se
    // borra ya y el job deja de quedarse "esperando" en Descargas.
    await discardInternalDownload(insp.archivePath, "cancelled");
  }
}
