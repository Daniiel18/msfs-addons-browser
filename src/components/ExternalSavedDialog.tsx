import { useEffect } from "react";
import { CheckCircle2, FolderOpen, Info, X } from "lucide-react";
import { t } from "../lib/i18n";
import { openSaved, useExternalSavedStore } from "../lib/externalSave";

/**
 * (v8.3) Aviso tras guardar contenido externo (app, instalador, navdata…) en
 * la carpeta del usuario, con las notas de instalación de la ficha (aerohub)
 * para que sepa qué hacer con el instalador. La carpeta ya se abrió sola.
 */
export function ExternalSavedDialog() {
  const info = useExternalSavedStore((s) => s.info);
  const close = useExternalSavedStore((s) => s.close);

  useEffect(() => {
    if (!info) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [info, close]);

  if (!info) return null;
  return (
    <div
      className="fixed inset-0 z-[65] flex items-center justify-center bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
      onClick={close}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-full w-full max-w-lg flex-col rounded-2xl border border-sf-border bg-sf-bg shadow-2xl"
      >
        <header className="flex items-start justify-between gap-3 border-b border-sf-border px-5 py-4">
          <div className="min-w-0">
            <h2 className="flex items-center gap-2 text-sm font-semibold text-sf-text">
              <CheckCircle2 className="h-4 w-4 text-emerald-400" />
              {t("external.saved", { name: info.name })}
            </h2>
            <p
              className="mt-1 select-text truncate font-mono text-[11px] text-sf-muted"
              title={info.path}
            >
              {info.path}
            </p>
            <p className="mt-1 text-xs text-amber-200/90">{t("external.saved_hint")}</p>
          </div>
          <button
            onClick={close}
            aria-label={t("common.close")}
            className="rounded-lg p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        {info.notes && (
          <div className="overflow-y-auto px-5 py-4">
            <h3 className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-sf-muted">
              <Info className="h-3.5 w-3.5 text-brand-300" />
              {t("result.install_notes")}
            </h3>
            <p className="select-text whitespace-pre-line break-words text-[13px] leading-relaxed text-sf-text">
              {info.notes}
            </p>
          </div>
        )}
        <footer className="flex justify-end gap-2 border-t border-sf-border bg-sf-surface px-5 py-3">
          <button
            onClick={() => void openSaved(info.path, info.isFile)}
            className="inline-flex items-center gap-1.5 rounded-md border border-sf-border px-3 py-1.5 text-xs text-sf-text hover:bg-sf-raised"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("external.open_folder")}
          </button>
          <button
            onClick={close}
            className="rounded-md bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400"
          >
            {t("common.close")}
          </button>
        </footer>
      </div>
    </div>
  );
}
