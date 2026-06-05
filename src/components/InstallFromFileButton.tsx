import { FileUp, Loader2 } from "lucide-react";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { t } from "../lib/i18n";

/**
 * Botón del header que dispara «Instalar desde archivo…».
 *
 * Pensado para los casos en los que el usuario ya tiene el .zip / .rar
 * / .7z en disco — sea porque lo descargó por su cuenta (mirror, otra
 * fuente) o porque quiere reinstalar algo que ya estaba en local. La
 * lógica de selector + extracción + copia a Community vive en el
 * store; aquí sólo mostramos el estado.
 */
export function InstallFromFileButton() {
  const busy = useDownloadsStore((s) => s.manualInstallBusy);
  const error = useDownloadsStore((s) => s.manualInstallError);
  const run = useDownloadsStore((s) => s.installFromFile);

  return (
    <div className="relative">
      <button
        onClick={() => run()}
        disabled={busy}
        className="inline-flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900 px-3 py-1.5 text-xs font-medium text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-60"
        title={t("install.from_file_tip")}
      >
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <FileUp className="h-3.5 w-3.5" />
        )}
        <span>Instalar desde archivo</span>
      </button>
      {/* Tooltip-error inline: aparece cuando la última instalación
          manual falló. Lo dejamos colgando del botón para que no
          ocupe espacio del layout cuando todo va bien. */}
      {error && !busy && (
        <div className="absolute right-0 top-full z-20 mt-2 w-72 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-[11px] text-red-200 shadow-lg">
          {error}
        </div>
      )}
    </div>
  );
}
