import { create } from "zustand";
import type { DropInspection } from "../lib/types";
import type { DropRouting } from "../components/DragDropOverlay";

/**
 * (Ajustes 19/10) Cola de instalación.
 *
 * Antes, si había un modal de instalación abierto y llegaba otro archivo
 * (arrastrar y soltar o una descarga del navegador embebido), el nuevo
 * REEMPLAZABA al anterior y esa instalación se perdía. Y un clic fuera del
 * modal cancelaba la instalación sin vuelta atrás.
 *
 * Ahora:
 *   · `active`  → el lote que muestra el modal (uno a la vez).
 *   · `queue`   → lotes esperando turno; se muestran al cerrar el actual.
 *   · `pending` → lotes "aparcados" (clic fuera / Escape / X). Viven en la
 *     campana de notificaciones con Instalar / Cancelar; clic en el aviso
 *     reabre el modal.
 *
 * Las sesiones de extracción del backend se conservan mientras el lote esté
 * en cola o pendiente (sólo se cancelan al cancelar de verdad).
 */
export interface InstallBatch {
  id: string;
  inspections: DropInspection[];
  routing: Map<string, DropRouting>;
  createdAt: number;
}

interface InstallQueueState {
  active: InstallBatch | null;
  queue: InstallBatch[];
  pending: InstallBatch[];
  /** Muestra el lote ya si no hay modal abierto; si no, lo pone en cola. */
  enqueue: (batch: InstallBatch) => void;
  /** El modal terminó (instalado o cancelado) → pasa al siguiente. */
  finishActive: () => void;
  /** Aparca el lote activo en pendientes → pasa al siguiente. */
  deferActive: () => void;
  /** Reabre un pendiente en el modal (o justo después del actual). */
  openPending: (id: string) => void;
  /** Saca un pendiente de la lista (para instalarlo o cancelarlo). */
  takePending: (id: string) => InstallBatch | null;
}

export const useInstallQueueStore = create<InstallQueueState>((set, get) => ({
  active: null,
  queue: [],
  pending: [],

  enqueue: (batch) =>
    set((s) => (s.active ? { queue: [...s.queue, batch] } : { active: batch })),

  finishActive: () =>
    set((s) => {
      const [next, ...rest] = s.queue;
      return { active: next ?? null, queue: rest };
    }),

  deferActive: () =>
    set((s) => {
      if (!s.active) return {};
      const [next, ...rest] = s.queue;
      return {
        pending: [...s.pending, s.active],
        active: next ?? null,
        queue: rest,
      };
    }),

  openPending: (id) => {
    const batch = get().takePending(id);
    if (!batch) return;
    set((s) => (s.active ? { queue: [batch, ...s.queue] } : { active: batch }));
  },

  takePending: (id) => {
    const batch = get().pending.find((b) => b.id === id) ?? null;
    if (batch) set((s) => ({ pending: s.pending.filter((b) => b.id !== id) }));
    return batch;
  },
}));

/** Nombre corto del lote para avisos: primer archivo (+N). */
export function batchLabel(batch: InstallBatch): string {
  const names = batch.inspections.map(
    (i) => i.archivePath.split(/[\\/]/).pop() ?? i.archivePath,
  );
  return names.length > 1 ? `${names[0]} (+${names.length - 1})` : names[0] ?? "";
}

/** Nº total de elementos instalables del lote. */
export function batchItemCount(batch: InstallBatch): number {
  return batch.inspections.reduce((n, i) => n + i.items.length, 0);
}
