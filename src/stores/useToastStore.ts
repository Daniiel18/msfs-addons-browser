import { create } from "zustand";

/**
 * Notificaciones efímeras (toasts) para feedback inmediato:
 *   · "info"    — descarga arrancada, manual ejecutándose…
 *   · "success" — instalación completada con éxito.
 *   · "error"   — fallo de descarga o instalación.
 *
 * El componente `<Toaster />` montado globalmente en `App` lee
 * `toasts` y los renderiza con animación. Cada toast se auto-elimina
 * después de `ttlMs` (default 4500ms para info/success, 8000 para
 * errores que el usuario debería leer).
 *
 * Diseño deliberado: no requiere `useToast()` hook ni provider.
 * Cualquier código (stores, components) puede llamar
 * `useToastStore.getState().push({...})` directamente — útil cuando
 * el evento que dispara el toast viene del WS de descargas y no
 * tiene un componente React encima.
 */

export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: string;
  kind: ToastKind;
  title: string;
  message?: string;
  /** Si está presente, el toast es clicable y dispara este callback. */
  onClick?: () => void;
  /** Tiempo en ms antes de auto-cerrar. `null` = persistente. */
  ttlMs: number | null;
}

interface ToastInput {
  kind: ToastKind;
  title: string;
  message?: string;
  onClick?: () => void;
  ttlMs?: number | null;
}

interface ToastState {
  toasts: Toast[];
  push: (t: ToastInput) => string;
  dismiss: (id: string) => void;
  clear: () => void;
}

let counter = 0;
const nextId = () => `toast-${Date.now()}-${counter++}`;

function defaultTtl(kind: ToastKind): number | null {
  if (kind === "error") return 8000;
  return 4500;
}

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],
  push(t) {
    const id = nextId();
    const toast: Toast = {
      id,
      kind: t.kind,
      title: t.title,
      message: t.message,
      onClick: t.onClick,
      ttlMs: t.ttlMs ?? defaultTtl(t.kind),
    };
    set((s) => ({ toasts: [...s.toasts, toast] }));
    if (toast.ttlMs !== null) {
      setTimeout(() => get().dismiss(id), toast.ttlMs);
    }
    return id;
  },
  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
  clear() {
    set({ toasts: [] });
  },
}));
