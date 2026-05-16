import { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  AlertCircle,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  Loader2,
  Sparkles,
  X,
} from "lucide-react";
import type { UpdateInfo } from "../lib/types";
import { api } from "../lib/tauri";

const DISMISSED_KEY = "msfs-addons-browser:updater:dismissed-version";

/**
 * Banner amarillo en la parte superior de la app que avisa cuando
 * GitHub Releases publicó una versión mayor a la instalada. Se
 * muestra una sola vez por versión: si el usuario lo cierra, queda
 * registrado en `localStorage` con la versión dismissed para no
 * repetir el aviso. Si más adelante hay una versión aún más nueva,
 * el banner reaparece (porque la `latestVersion` cambia).
 *
 * El chequeo se dispara una vez al montar y no insiste — esto es
 * intencional: la UX que queremos es "te aviso al arrancar" no
 * "te molesto cada minuto".
 */
export function UpdateBanner() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [hidden, setHidden] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<{
    downloadedBytes: number;
    totalBytes: number | null;
  } | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const unsubProgressRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .checkForUpdate()
      .then((u) => {
        if (cancelled || !u) return;
        // Si el usuario ya descartó esta misma versión, no reabrimos
        // el banner — sólo cuando salga una nueva mayor.
        const dismissed = localStorage.getItem(DISMISSED_KEY);
        if (dismissed === u.latestVersion) return;
        setInfo(u);
      })
      .catch((e) => console.warn("checkForUpdate failed:", e));
    return () => {
      cancelled = true;
    };
  }, []);

  // Cleanup del listener al desmontar.
  useEffect(() => {
    return () => {
      unsubProgressRef.current?.();
    };
  }, []);

  if (!info || hidden) return null;

  const dismiss = () => {
    localStorage.setItem(DISMISSED_KEY, info.latestVersion);
    setHidden(true);
  };

  const installNow = async () => {
    if (!info.assetUrl) return;
    setInstalling(true);
    setInstallError(null);
    setProgress({ downloadedBytes: 0, totalBytes: null });
    // Suscribir al progreso antes de empezar — evita perder los
    // primeros chunks si la conexión es muy rápida.
    try {
      unsubProgressRef.current = await api.onUpdateProgress((p) =>
        setProgress(p),
      );
    } catch (e) {
      console.warn("no se pudo suscribir a updater://progress:", e);
    }
    try {
      await api.installUpdate(info.assetUrl);
      // Si la llamada vuelve sin error, el backend está a punto de
      // hacer exit(0). Dejamos la barra en "Lanzando installer…" hasta
      // que la app muera.
      setProgress((prev) =>
        prev ? { ...prev, totalBytes: prev.downloadedBytes } : null,
      );
    } catch (e) {
      setInstallError(String(e));
      setInstalling(false);
      unsubProgressRef.current?.();
      unsubProgressRef.current = null;
    }
  };

  const lines = parseMarkdownLines(info.notesMarkdown);
  const pct =
    progress && progress.totalBytes && progress.totalBytes > 0
      ? Math.min(
          100,
          Math.round((progress.downloadedBytes / progress.totalBytes) * 100),
        )
      : null;
  const mbDown = progress ? (progress.downloadedBytes / 1_048_576).toFixed(1) : null;
  const mbTotal =
    progress?.totalBytes != null
      ? (progress.totalBytes / 1_048_576).toFixed(1)
      : null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, y: -16 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -16 }}
        className="border-b border-emerald-500/30 bg-emerald-500/10 text-emerald-100"
      >
        <div className="mx-auto flex max-w-5xl flex-col gap-2 px-6 py-3 text-sm">
          <div className="flex items-center gap-3">
            <Sparkles className="h-4 w-4 shrink-0 text-emerald-300" />
            <div className="flex-1">
              <span className="font-medium">Hay una nueva versión disponible</span>
              <span className="ml-2 text-emerald-200/70">
                {info.currentVersion} → <strong>{info.latestVersion}</strong>
              </span>
            </div>
            {lines.length > 0 && (
              <button
                onClick={() => setExpanded((v) => !v)}
                className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-emerald-200 hover:bg-emerald-500/20"
              >
                {expanded ? (
                  <>
                    <ChevronUp className="h-3.5 w-3.5" /> Ocultar notas
                  </>
                ) : (
                  <>
                    <ChevronDown className="h-3.5 w-3.5" /> Ver notas
                  </>
                )}
              </button>
            )}
            {info.assetUrl ? (
              <button
                onClick={installNow}
                disabled={installing}
                className="inline-flex items-center gap-1.5 rounded-md bg-emerald-500/30 px-3 py-1 text-xs font-medium hover:bg-emerald-500/40 disabled:opacity-60"
                title="Descarga e instala automáticamente — la app se cerrará para que el installer reemplace los archivos"
              >
                {installing ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Download className="h-3.5 w-3.5" />
                )}
                {installing ? "Instalando…" : "Instalar ahora"}
              </button>
            ) : (
              <button
                onClick={() => api.openExternal(info.releaseUrl)}
                className="inline-flex items-center gap-1.5 rounded-md bg-emerald-500/30 px-3 py-1 text-xs font-medium hover:bg-emerald-500/40"
              >
                <ExternalLink className="h-3.5 w-3.5" /> Ver release
              </button>
            )}
            <button
              onClick={dismiss}
              disabled={installing}
              title="No volver a avisarme sobre esta versión"
              className="rounded-md p-1 text-emerald-200/70 hover:bg-emerald-500/20 hover:text-emerald-100 disabled:opacity-50"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* Progress bar + estado mientras la descarga corre. */}
          {installing && progress && (
            <div className="ml-7 mt-1 space-y-1">
              <div className="flex items-center justify-between text-[10px] text-emerald-200/80">
                <span>
                  {pct !== null
                    ? `Descargando ${pct}%`
                    : `Descargando ${mbDown ?? "0"} MB`}
                </span>
                <span className="font-mono tabular-nums">
                  {mbDown ?? "0"} {mbTotal ? `/ ${mbTotal}` : ""} MB
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-emerald-500/20">
                <div
                  className="h-full bg-emerald-400 transition-all duration-150"
                  style={{
                    width:
                      pct !== null
                        ? `${pct}%`
                        : // Indeterminate: mostrar barra pulsante al 30%.
                          "30%",
                  }}
                />
              </div>
              {pct === 100 && (
                <div className="text-[10px] text-emerald-200/80">
                  Descarga completa — lanzando el instalador. La app se cerrará
                  en unos segundos.
                </div>
              )}
            </div>
          )}

          {installError && (
            <div className="ml-7 mt-1 flex items-start gap-1.5 rounded bg-rose-500/15 px-2 py-1 text-[11px] text-rose-200 ring-1 ring-rose-500/30">
              <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
              <span>{installError}</span>
            </div>
          )}

          <AnimatePresence initial={false}>
            {expanded && lines.length > 0 && (
              <motion.ul
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.18 }}
                className="ml-7 list-disc space-y-0.5 overflow-hidden pl-3 text-xs text-emerald-100/90"
              >
                {lines.map((line, i) => (
                  <li key={i}>{line}</li>
                ))}
              </motion.ul>
            )}
          </AnimatePresence>
        </div>
      </motion.div>
    </AnimatePresence>
  );
}

/**
 * Convierte el cuerpo Markdown de una release de GitHub en una
 * lista plana de líneas legibles. Es deliberadamente simple — no
 * queremos un parser completo de Markdown en el bundle:
 *
 *   · `## Heading` → línea sola, sin `##`
 *   · `- bullet` / `* bullet` → línea con bullet quitado
 *   · `**bold**` / `*italic*` / backticks → texto plano
 *   · líneas vacías o sólo whitespace se descartan
 *
 * Es un puerto del `ChangelogParser` del .NET legacy, adaptado a
 * Markdown en lugar de HTML (GitHub guarda `body` en Markdown).
 */
function parseMarkdownLines(md: string): string[] {
  if (!md.trim()) return [];
  const out: string[] = [];
  for (const raw of md.split("\n")) {
    let line = raw.trim();
    if (!line) continue;
    // Headings: quita los `#` iniciales
    line = line.replace(/^#+\s*/, "");
    // Bullets: quita marcadores `-`, `*`, `+`
    line = line.replace(/^[-*+]\s+/, "");
    // Quita énfasis básico (**bold**, *italic*, _italic_, `code`)
    line = line
      .replace(/\*\*(.+?)\*\*/g, "$1")
      .replace(/\*(.+?)\*/g, "$1")
      .replace(/_(.+?)_/g, "$1")
      .replace(/`(.+?)`/g, "$1");
    if (line.length > 1) out.push(line);
  }
  return out;
}
