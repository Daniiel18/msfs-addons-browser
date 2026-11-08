import { useEffect } from "react";
import { createPortal } from "react-dom";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ExternalLink, FolderOpen, X } from "lucide-react";
import type { LandingClip } from "../lib/types";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (8.4.x) Reproductor del video de aterrizaje dentro de SimFleet (desde el
 * FlightBook), con "Mostrar en la carpeta" y "Abrir en el reproductor".
 */
export function ClipPlayerModal({
  clip,
  title,
  onClose,
}: {
  clip: LandingClip;
  /** Ruta del vuelo ("KSFO → CYYC") o similar. */
  title: string;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  let src: string | null = null;
  if (isTauri) {
    try {
      src = convertFileSrc(clip.path);
    } catch {
      src = null;
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-6" onClick={onClose}>
      <div
        data-sf-overlay
        onClick={(e) => e.stopPropagation()}
        className="flex w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl"
      >
        <header className="flex items-center gap-3 border-b border-sf-border px-4 py-3">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-sf-text">{t("fb.clip.title")}</div>
            <div className="truncate font-mono text-[11px] text-sf-muted">
              {title}
              {clip.fpm != null ? ` · ${-Math.abs(clip.fpm)} fpm` : ""}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <div className="bg-black">
          {src ? (
            <video src={src} controls autoPlay playsInline className="aspect-video w-full" />
          ) : (
            <div className="flex aspect-video items-center justify-center text-xs text-sf-muted">
              {t("fb.clip.unavailable")}
            </div>
          )}
        </div>
        <footer className="flex items-center justify-end gap-2 px-4 py-3">
          <button
            type="button"
            onClick={() => void api.revealPath(clip.path, true).catch(() => {})}
            className="inline-flex items-center gap-1.5 rounded-lg border border-sf-border px-3 py-1.5 text-xs text-sf-text/90 hover:border-brand-500/40"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("fb.clip.show_folder")}
          </button>
          <button
            type="button"
            onClick={() => void api.openLocalPath(clip.path).catch(() => {})}
            className="inline-flex items-center gap-1.5 rounded-lg border border-sf-border px-3 py-1.5 text-xs text-sf-text/90 hover:border-brand-500/40"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            {t("fb.clip.open_external")}
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
