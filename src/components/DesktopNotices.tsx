import { useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import { BrandMark } from "./BrandMark";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";

/**
 * (23/10) Avisos de escritorio con el diseño de SimFleet. Corre en su propia
 * ventana ("notify", transparente y siempre encima) que crea el backend
 * (`desktop_notice.rs`); `main.tsx` enruta por label.
 *
 * Sin IPC: el backend pinta la lista llamando a `window.__sfNotices(list)`,
 * y los clics navegan a `https://sfnotice.local/{open|close}/{id}`, que el
 * backend intercepta (abre SimFleet / cierra el aviso) y cancela.
 *
 * Las medidas (alto de tarjeta 76 px, separación 8, margen 10) coinciden con
 * las del backend, que ajusta el alto de la ventana a los avisos.
 */
interface Notice {
  id: number;
  title: string;
  body: string | null;
  kind: "info" | "success" | "error";
}

declare global {
  interface Window {
    __sfNotices?: (list: Notice[]) => void;
  }
}

const KIND = {
  info: { icon: Info, bar: "bg-brand-400", tint: "text-brand-300" },
  success: { icon: CheckCircle2, bar: "bg-emerald-400", tint: "text-emerald-300" },
  error: { icon: AlertTriangle, bar: "bg-rose-400", tint: "text-rose-300" },
} as const;

function go(action: "open" | "close", id: number) {
  window.location.href = `https://sfnotice.local/${action}/${id}`;
}

export function DesktopNotices() {
  const [list, setList] = useState<Notice[]>([]);
  useEffect(() => {
    // Ventana transparente: sólo se ven las tarjetas.
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    window.__sfNotices = (l) => setList(Array.isArray(l) ? l : []);
    return () => {
      delete window.__sfNotices;
    };
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col justify-end gap-2 overflow-hidden bg-transparent p-[10px]">
      {list.map((n) => {
        const k = KIND[n.kind] ?? KIND.info;
        const Icon = k.icon;
        return (
          <div
            key={n.id}
            onClick={() => go("open", n.id)}
            title={t("notice.open_hint")}
            className="relative flex h-[76px] shrink-0 cursor-pointer items-start gap-3 overflow-hidden rounded-xl border border-sf-border bg-sf-surface/95 py-2.5 pl-4 pr-8 shadow-lg shadow-black/40 transition-colors hover:border-brand-500/40"
          >
            <span className={cn("absolute inset-y-0 left-0 w-1", k.bar)} />
            <BrandMark className="mt-0.5 h-8 w-8 shrink-0 rounded-lg" />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">
                <Icon className={cn("h-3 w-3", k.tint)} />
                SimFleet
              </div>
              <div className="truncate text-[13px] font-semibold text-sf-text">{n.title}</div>
              {n.body && <div className="truncate text-[11px] text-sf-muted">{n.body}</div>}
            </div>
            <button
              onClick={(e) => {
                e.stopPropagation();
                go("close", n.id);
              }}
              title={t("notice.close")}
              className="absolute right-2 top-2 rounded p-0.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
