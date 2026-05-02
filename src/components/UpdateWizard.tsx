import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  Check,
  Download,
  Loader2,
  SkipForward,
  Sparkles,
  X,
} from "lucide-react";
import type { Addon, DownloadMethod } from "../lib/types";
import { api } from "../lib/tauri";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useDownloadsStore } from "../stores/useDownloadsStore";

/**
 * Wizard global que itera la cola `updateQueue` del community store
 * y muestra para cada item los métodos de descarga disponibles. El
 * usuario:
 *   · Elige un método → desinstalamos el paquete actual + lanzamos
 *     la descarga (que reinstala). Pasa al siguiente.
 *   · Salta → no toca el paquete, sólo avanza al siguiente.
 *   · Cancela → vacía la cola.
 *
 * El componente se monta una sola vez en `App` y se renderiza sólo
 * cuando hay items en cola.
 */
export function UpdateWizard() {
  const queue = useCommunityStore((s) => s.updateQueue);
  const shift = useCommunityStore((s) => s.shiftUpdateQueue);
  const cancel = useCommunityStore((s) => s.cancelUpdateQueue);
  const rescan = useCommunityStore((s) => s.rescan);
  const startDownload = useDownloadsStore((s) => s.start);

  const current = queue[0] ?? null;
  const total = queue.length;
  // El total inicial se congela al abrir el wizard para que el
  // contador "1 de 5" sea estable mientras avanzamos.
  const [initialTotal, setInitialTotal] = useState(total);
  useEffect(() => {
    if (total > initialTotal) setInitialTotal(total);
    if (total === 0) setInitialTotal(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [total]);

  const [matches, setMatches] = useState<Addon[]>([]);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Lazy-load del catálogo cada vez que cambia el item actual.
  useEffect(() => {
    if (!current) return;
    let cancelled = false;
    setMatches([]);
    setError(null);
    setSearching(true);
    Promise.all([
      api.search(current.icao, "sceneryaddons").catch(() => [] as Addon[]),
      api.search(current.icao, "simplaza").catch(() => [] as Addon[]),
    ])
      .then(([sa, sp]) => {
        if (!cancelled) setMatches([...sa, ...sp]);
      })
      .finally(() => {
        if (!cancelled) setSearching(false);
      });
    return () => {
      cancelled = true;
    };
  }, [current?.folderName]); // eslint-disable-line react-hooks/exhaustive-deps

  // Filtro estricto por el addon específico (no todos los
  // desarrolladores). Cuando el usuario tiene "LFPG Charles de
  // Gaulle (Merged)" instalado y busca update, sólo nos interesa
  // ese desarrollador concreto — no inundar con todas las
  // variantes "(Converted)", "(LSS Edit)", etc. de otros devs.
  //
  // Heurística:
  //   1. Match exacto por `addon.id === current.addonId` (el
  //      AvailableUpdate viene del JOIN con la fila concreta del
  //      catálogo que disparó el aviso).
  //   2. Fallback por título normalizado (mismo creator/variante).
  //   3. Last resort: todos los que comparten ICAO.
  const grouped = useMemo(() => {
    if (!current) return [];
    const exactById = matches.filter((m) => m.id === current.addonId);
    if (exactById.length > 0) {
      return groupBySourceWithMethods(exactById, current.icao);
    }
    const targetTitle = normalizeTitle(current.title);
    const sameTitle = matches.filter(
      (m) => normalizeTitle(m.name) === targetTitle,
    );
    if (sameTitle.length > 0) {
      return groupBySourceWithMethods(sameTitle, current.icao);
    }
    return groupBySourceWithMethods(matches, current.icao);
  }, [matches, current]);

  const pickMethod = async (addon: Addon, method: DownloadMethod) => {
    if (!current) return;
    setBusy(true);
    setError(null);
    try {
      // El flujo es el mismo que "Reparar limpio": desinstala +
      // descarga + instala. La instalación borra el folder previo
      // antes de copiar, así que la sobrescritura es consistente.
      await api.uninstallCommunityPackage(current.folderName);
      await startDownload({
        addonId: addon.id,
        addonTitle: addon.name,
        source: addon.source,
        method,
      });
      shift();
      rescan().catch(() => {});
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const skip = () => {
    setError(null);
    shift();
  };

  // ESC cancela el wizard entero (con confirm implícito: la cola
  // se limpia, las updates siguen en notificaciones).
  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [current, busy, cancel]);

  return (
    <AnimatePresence>
      {current && (
        <motion.div
          key="wizard-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80 backdrop-blur-sm"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 8 }}
            transition={{ duration: 0.16 }}
            className="w-[min(640px,calc(100vw-2rem))] max-h-[calc(100vh-3rem)] overflow-hidden rounded-2xl border border-amber-500/40 bg-slate-950 shadow-2xl ring-1 ring-amber-500/20"
          >
            <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-[11px] uppercase tracking-wide text-amber-300">
                  <Sparkles className="h-3.5 w-3.5" />
                  Asistente de actualización
                  <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-200">
                    {initialTotal - total + 1} / {initialTotal}
                  </span>
                </div>
                <h2 className="mt-1.5 truncate text-base font-semibold text-slate-100">
                  {current.title}
                </h2>
                <p className="mt-0.5 text-xs text-slate-400">
                  <span className="font-mono text-emerald-300">{current.icao}</span>
                  {" · "}
                  <span className="text-slate-300">v{current.installedVersion}</span>
                  {" → "}
                  <span className="font-medium text-amber-200">v{current.latestVersion}</span>
                </p>
              </div>
              <button
                onClick={cancel}
                disabled={busy}
                className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
                title="Cancelar el asistente (las updates pendientes siguen en notificaciones)"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            <div className="max-h-[60vh] overflow-y-auto p-4">
              {error && (
                <div className="mb-3 flex items-start gap-2 rounded bg-rose-500/15 px-3 py-2 text-xs text-rose-200 ring-1 ring-rose-500/30">
                  <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>{error}</span>
                </div>
              )}

              {searching ? (
                <div className="flex items-center justify-center gap-2 py-8 text-xs text-slate-400">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Buscando {current.icao} en SceneryAddons y Simplaza…
                </div>
              ) : grouped.length === 0 ? (
                <p className="py-6 text-center text-xs text-slate-500">
                  No hay métodos de descarga disponibles. Pulsa "Saltar" para
                  pasar al siguiente.
                </p>
              ) : (
                <ul className="space-y-3">
                  {grouped.map((g, gi) => (
                    <li
                      key={`${g.source}-${gi}`}
                      className="rounded-lg border border-slate-800 bg-slate-900/50 p-3"
                    >
                      <div className="mb-1.5 flex items-center gap-2 text-[11px]">
                        <span className="rounded bg-slate-800 px-1.5 py-0.5 font-semibold uppercase tracking-wide text-slate-300">
                          {g.source}
                        </span>
                        {g.version && <span className="text-slate-500">v{g.version}</span>}
                      </div>
                      <p className="mb-2 truncate text-xs text-slate-200">{g.title}</p>
                      <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                        {g.methods.map((mm, mi) => (
                          <button
                            key={mi}
                            onClick={() => pickMethod(g.addon, mm)}
                            disabled={busy}
                            className={`inline-flex items-center gap-2 rounded-md border px-3 py-2 text-left text-xs font-medium transition-colors disabled:opacity-50 ${
                              mm.kind === "torrent"
                                ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-200 hover:border-emerald-400 hover:bg-emerald-500/20"
                                : "border-slate-700 bg-slate-800/60 text-slate-200 hover:border-brand-500/40 hover:bg-slate-800"
                            }`}
                          >
                            {busy ? (
                              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                            ) : (
                              <Download className="h-3.5 w-3.5 shrink-0" />
                            )}
                            <span className="min-w-0 flex-1 truncate">{mm.name}</span>
                            <span className="shrink-0 rounded bg-slate-900/60 px-1.5 py-0.5 text-[10px] uppercase">
                              {mm.kind}
                            </span>
                          </button>
                        ))}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            <footer className="flex items-center justify-between gap-2 border-t border-slate-800 px-5 py-3 text-xs">
              <button
                onClick={skip}
                disabled={busy}
                className="inline-flex items-center gap-1.5 rounded-md border border-slate-700 px-3 py-1.5 text-slate-300 hover:bg-slate-800 disabled:opacity-50"
              >
                <SkipForward className="h-3.5 w-3.5" />
                Saltar
              </button>
              <span className="text-slate-500">
                {total - 1 > 0
                  ? `${total - 1} más después de éste`
                  : "Última actualización"}
              </span>
              <button
                onClick={cancel}
                disabled={busy}
                className="inline-flex items-center gap-1.5 rounded-md border border-slate-700 px-3 py-1.5 text-slate-300 hover:bg-slate-800 disabled:opacity-50"
              >
                <Check className="h-3.5 w-3.5" />
                Terminar
              </button>
            </footer>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

interface Grouped {
  source: string;
  addon: Addon;
  title: string;
  version: string | null;
  methods: DownloadMethod[];
}

function groupBySourceWithMethods(matches: Addon[], icao: string): Grouped[] {
  const target = icao.toUpperCase();
  // Filtramos al ICAO exacto para evitar mostrar resultados de
  // otros aeropuertos cuando la búsqueda devuelve mucha cosa.
  const filtered = target
    ? matches.filter((m) => m.icao?.toUpperCase() === target)
    : matches;
  // Agrupamos por (source, addonId) para preservar la integridad
  // de las opciones — un mismo addon en varias versiones aparece
  // como filas separadas.
  return filtered
    .filter((m) => m.downloadMethods.length > 0)
    .sort((a, b) => (b.version ?? "").localeCompare(a.version ?? ""))
    .map((m) => ({
      source: m.source,
      addon: m,
      title: m.name,
      version: m.version,
      methods: orderMethods(m.downloadMethods),
    }));
}

/** Normalización igual a la del ResultCard — debe coincidir
 *  para que el filtro de instalación y el filtro del wizard usen
 *  la misma noción de "este addon es el mismo". */
function normalizeTitle(s: string): string {
  return s
    .toLowerCase()
    .replace(/\sv\d+(?:\.\d+)+\b.*$/i, "")
    .replace(/\bmsfs\s*\d{4}(?:\/\d+)?\b/gi, "")
    .replace(/[\s ]+/g, " ")
    .trim();
}

function orderMethods(methods: DownloadMethod[]): DownloadMethod[] {
  const order: Record<string, number> = { torrent: 0, mirror: 1, direct: 2 };
  return [...methods].sort(
    (a, b) => (order[a.kind] ?? 9) - (order[b.kind] ?? 9),
  );
}
