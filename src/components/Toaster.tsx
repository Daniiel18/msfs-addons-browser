import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, CheckCircle2, Download, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useToastStore, type Toast } from "../stores/useToastStore";
import { t as tr } from "../lib/i18n";
import { isTauri } from "../lib/tauri";
import { useFstoEmbedStore } from "./FlightsimEmbed";

/**
 * Renderizador global de toasts. Pinta una columna en la esquina
 * inferior derecha. Cada toast entra deslizándose desde la derecha
 * y sale fundiéndose. `framer-motion` se encarga del enter/exit
 * automáticamente — sólo enchufamos el array del store.
 */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  // (23/10) flightsim.to es un webview NATIVO encima de la app: tapa estos
  // toasts HTML (p. ej. "descarga iniciada"). Mientras se ve, cada toast nuevo
  // sale también en la ventana de avisos de SimFleet, que queda encima, anclada
  // a la esquina de la app. El toast HTML sigue ahí para cuando se oculte.
  const forwarded = useRef(new Set<string>());
  useEffect(() => {
    if (!isTauri) return;
    for (const t of toasts) {
      if (forwarded.current.has(t.id)) continue;
      forwarded.current.add(t.id);
      if (!useFstoEmbedStore.getState().visible) continue;
      void invoke("desktop_notice", {
        title: t.title,
        body: t.message ?? null,
        kind: t.kind,
        anchor: "app",
      }).catch(() => {});
    }
    const alive = new Set(toasts.map((t) => t.id));
    for (const id of forwarded.current) if (!alive.has(id)) forwarded.current.delete(id);
  }, [toasts]);

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[70] flex w-[min(380px,calc(100vw-2rem))] flex-col gap-2">
      <AnimatePresence>
        {toasts.map((t) => (
          <ToastItem key={t.id} toast={t} onClose={() => dismiss(t.id)} />
        ))}
      </AnimatePresence>
    </div>
  );
}

function ToastItem({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  const palette = palettes[toast.kind];

  const Icon = palette.Icon;

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: 80, scale: 0.95 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, scale: 0.92, transition: { duration: 0.2 } }}
      transition={{ type: "spring", stiffness: 400, damping: 30 }}
      className={`pointer-events-auto flex items-start gap-3 overflow-hidden rounded-xl border bg-sf-surface p-3 shadow-xl shadow-black/30 ring-1 ${palette.ring}`}
    >
      <motion.div
        // Pulso sutil en el icono cuando entra. Da sensación de
        // "feedback" inmediato sin ser ruidoso.
        initial={{ scale: 0.6 }}
        animate={{ scale: 1 }}
        transition={{ type: "spring", stiffness: 500, damping: 18 }}
        className={`mt-0.5 shrink-0 ${palette.iconColor}`}
      >
        <Icon className="h-5 w-5" />
      </motion.div>
      <button
        onClick={() => toast.onClick?.()}
        disabled={!toast.onClick}
        className="min-w-0 flex-1 text-left disabled:cursor-default"
      >
        <p className={`text-sm font-medium ${palette.titleColor}`}>{toast.title}</p>
        {toast.message && (
          <p className="mt-0.5 text-xs text-sf-muted">{toast.message}</p>
        )}
      </button>
      <button
        onClick={onClose}
        className="shrink-0 rounded-md p-1 text-sf-muted transition-colors hover:bg-sf-raised hover:text-sf-text"
        title={tr("common.close")}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </motion.div>
  );
}

const palettes = {
  info: {
    Icon: Download,
    ring: "ring-brand-500/40 border-brand-500/30",
    iconColor: "text-brand-300",
    titleColor: "text-sf-text",
  },
  success: {
    Icon: CheckCircle2,
    ring: "ring-emerald-500/40 border-emerald-500/30",
    iconColor: "text-emerald-300",
    titleColor: "text-emerald-100",
  },
  error: {
    Icon: AlertCircle,
    ring: "ring-rose-500/40 border-rose-500/30",
    iconColor: "text-rose-300",
    titleColor: "text-rose-100",
  },
} as const;
