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
import { t } from "../lib/i18n";
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
    // (v3.1.3 / v3.3.0) Polling cada 15 min + check inicial + check
    // en `window.focus`. El usuario reportó que con polling 1h el
    // banner se demoraba en aparecer cuando publicaba un release y
    // volvía a la app. Ahora:
    //   1. Check al montar (mismo).
    //   2. Interval 15 min (bajado de 1h).
    //   3. Trigger inmediato en window.focus — cuando el usuario
    //      vuelve a la app tras estar en otro lugar (navegador,
    //      Discord, etc.) se chequea contra GitHub al instante.
    const check = () => {
      api
        .checkForUpdate()
        .then((u) => {
          if (cancelled || !u) return;
          const dismissed = localStorage.getItem(DISMISSED_KEY);
          if (dismissed === u.latestVersion) return;
          setInfo(u);
          setHidden(false);
        })
        .catch((e) => console.warn("checkForUpdate failed:", e));
    };
    check();
    const interval = setInterval(check, 15 * 60_000);
    const onFocus = () => {
      if (cancelled) return;
      check();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      clearInterval(interval);
      window.removeEventListener("focus", onFocus);
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
        // (v2.2.0) Banner re-diseñado: más legible, responsive, con
        // jerarquía visual clara. El close X queda PROMINENTE arriba
        // a la derecha sin solaparse con los otros botones.
        className="border-b border-emerald-500/30 bg-gradient-to-r from-emerald-500/15 via-emerald-500/10 to-emerald-500/15 text-emerald-100"
      >
        <div className="mx-auto flex w-full max-w-6xl items-start gap-3 px-4 py-3 sm:px-6">
          <div className="mt-0.5 shrink-0 rounded-md bg-emerald-500/20 p-1.5 ring-1 ring-emerald-500/40">
            <Sparkles className="h-4 w-4 text-emerald-300" />
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
              <span className="text-sm font-semibold text-emerald-50">
                Nueva versión disponible
              </span>
              <span className="font-mono text-xs text-emerald-200/80">
                v{info.currentVersion} → v{info.latestVersion}
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {info.assetUrl ? (
                <button
                  onClick={installNow}
                  disabled={installing}
                  className="inline-flex items-center gap-1.5 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400 disabled:opacity-60"
                  title={t("update.auto_tip")}
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
                  className="inline-flex items-center gap-1.5 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400"
                >
                  <ExternalLink className="h-3.5 w-3.5" /> Ver en GitHub
                </button>
              )}
              {lines.length > 0 && (
                <button
                  onClick={() => setExpanded((v) => !v)}
                  className="inline-flex items-center gap-1 rounded-md border border-emerald-500/30 px-2.5 py-1.5 text-xs text-emerald-100 hover:bg-emerald-500/15"
                >
                  {expanded ? (
                    <>
                      <ChevronUp className="h-3.5 w-3.5" /> Ocultar cambios
                    </>
                  ) : (
                    <>
                      <ChevronDown className="h-3.5 w-3.5" /> Ver cambios ({lines.length})
                    </>
                  )}
                </button>
              )}
            </div>
          </div>
          <button
            onClick={dismiss}
            disabled={installing}
            title={t("update.later_tip")}
            className="ml-1 shrink-0 rounded-md border border-emerald-500/30 bg-emerald-500/10 p-1.5 text-emerald-100 hover:bg-emerald-500/25 disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="mx-auto w-full max-w-6xl px-4 sm:px-6">
          {/* Progress + notes wrappers heredados del bloque anterior */}

          {/* Progress bar + estado mientras la descarga corre. */}
          {installing && progress && (
            <div className="mb-2 mt-1 space-y-1 pl-9">
              <div className="flex items-center justify-between text-[11px] text-emerald-100">
                <span>
                  {pct !== null
                    ? `Descargando · ${pct}%`
                    : `Descargando ${mbDown ?? "0"} MB`}
                </span>
                <span className="font-mono tabular-nums">
                  {mbDown ?? "0"} {mbTotal ? `/ ${mbTotal}` : ""} MB
                </span>
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-emerald-500/20">
                <div
                  className="h-full bg-emerald-400 transition-all duration-150"
                  style={{
                    width: pct !== null ? `${pct}%` : "30%",
                  }}
                />
              </div>
              {pct === 100 && (
                <div className="text-[11px] text-emerald-200/80">
                  Descarga completa — lanzando el instalador. La app se reabre
                  en unos segundos.
                </div>
              )}
            </div>
          )}

          {installError && (
            <div className="mb-2 mt-1 flex items-start gap-1.5 rounded-md bg-rose-500/15 px-3 py-2 text-xs text-rose-200 ring-1 ring-rose-500/30">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{installError}</span>
            </div>
          )}

          <AnimatePresence initial={false}>
            {expanded && lines.length > 0 && (
              <motion.ul
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.2 }}
                className="mb-3 list-disc space-y-0.5 overflow-hidden rounded-md border border-emerald-500/20 bg-emerald-500/5 pl-7 pr-3 py-2 text-xs text-emerald-100/90"
              >
                {lines.slice(0, 30).map((line, i) => (
                  <li key={i}>{line}</li>
                ))}
                {lines.length > 30 && (
                  <li className="text-emerald-300/70">
                    + {lines.length - 30} más en{" "}
                    <button
                      onClick={() => api.openExternal(info.releaseUrl)}
                      className="underline"
                    >
                      GitHub
                    </button>
                  </li>
                )}
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
