import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { motion } from "framer-motion";
import { X } from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";

export interface DetailTab {
  id: string;
  label: string;
  icon?: ReactNode;
  badge?: string | number;
}

interface Props {
  title: string;
  subtitle?: ReactNode;
  icon?: ReactNode;
  tabs?: DetailTab[];
  tab?: string;
  onTab?: (id: string) => void;
  /** Botones a la derecha del título. */
  actions?: ReactNode;
  /** Barra fija abajo a la derecha. */
  footer?: ReactNode;
  /** Por encima de otro modal abierto. */
  zClass?: string;
  onClose: () => void;
  children: ReactNode;
}

/**
 * ¿`el` es la ventana de más arriba? Lo que se abre después queda detrás en el
 * documento, también lo que se abre dentro de otra ventana. Escape cierra sólo
 * esa, y los campos y visores de dentro pueden quedárselo antes.
 */
export function isTopOverlay(el: Element | null): boolean {
  const overlays = document.querySelectorAll(".fixed.inset-0:not(.pointer-events-none)");
  return !!el && overlays[overlays.length - 1] === el;
}

/**
 * (03/11) Limpieza de vistas: el detalle que alargaba una pantalla se abre
 * aquí. Mismo tamaño siempre, con pestañas opcionales; en portal para que no
 * lo recorten los paneles de la pantalla.
 */
export function DetailModal({
  title,
  subtitle,
  icon,
  tabs,
  tab,
  onTab,
  actions,
  footer,
  zClass = "z-[60]",
  onClose,
  children,
}: Props) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && isTopOverlay(root.current)) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return createPortal(
    <motion.div
      ref={root}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.12 }}
      onClick={onClose}
      className={cn("fixed inset-0 flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm", zClass)}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16 }}
        onClick={(e) => e.stopPropagation()}
        className="flex h-[min(720px,calc(100vh-3rem))] w-[min(1040px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl ring-1 ring-sf-border"
      >
        <header className="flex shrink-0 items-center gap-3 border-b border-sf-border px-5 py-3">
          {icon && <span className="shrink-0 text-brand-400">{icon}</span>}
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-sm font-semibold text-sf-text">{title}</h2>
            {subtitle && <p className="truncate text-xs text-sf-muted">{subtitle}</p>}
          </div>
          {actions}
          <button
            type="button"
            onClick={onClose}
            aria-label={t("common.close")}
            title={t("common.close")}
            className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        {tabs && tabs.length > 1 && (
          <nav className="flex shrink-0 gap-1 overflow-x-auto border-b border-sf-border px-3">
            {tabs.map((x) => (
              <button
                key={x.id}
                type="button"
                onClick={() => onTab?.(x.id)}
                className={cn(
                  "inline-flex shrink-0 items-center gap-1.5 border-b-2 px-3 py-2.5 text-xs font-medium",
                  x.id === tab
                    ? "border-brand-400 text-brand-300"
                    : "border-transparent text-sf-muted hover:text-sf-text",
                )}
              >
                {x.icon}
                {x.label}
                {x.badge !== undefined && (
                  <span className="rounded-full bg-amber-500/20 px-1.5 text-[11px] font-bold text-amber-300">{x.badge}</span>
                )}
              </button>
            ))}
          </nav>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto p-5">{children}</div>
        {footer && (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-sf-border px-5 py-3">
            {footer}
          </footer>
        )}
      </motion.div>
    </motion.div>,
    document.body,
  );
}
