import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  FileText,
  Loader2,
  Search as SearchIcon,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import type { Addon, SourceDescriptor } from "../lib/types";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { t } from "../lib/i18n";

/**
 * (v1.1.4) Modal para importar un inventario exportado y elegir
 * qué descargar. El usuario pidió: tras formatear el PC, importa
 * el inventario y selecciona los addons que quiere re-instalar.
 *
 * Flujo:
 *   1. Lee el archivo (CSV/TXT/JSON) y extrae títulos/ICAOs/desarrolladores.
 *   2. Para cada item, busca en SceneryAddons y Simplaza el match
 *      más cercano (por ICAO si existe, sino por título).
 *   3. Lista los matches agrupados por categoría (Aeropuertos /
 *      Aviones / Otros) con checkbox para seleccionar.
 *   4. "Continuar" muestra los métodos de descarga disponibles y
 *      añade todo a la cola.
 *
 * Limitaciones:
 *   · El match no es exacto; el usuario puede destildar items que
 *     no coincidan.
 *   · Si la búsqueda devuelve 0 resultados para un item, se muestra
 *     en gris como "No encontrado".
 */

interface InventoryItem {
  /** Nombre original tal como aparece en el export. */
  rawTitle: string;
  icao?: string;
  developer?: string;
  version?: string;
}

interface ResolvedItem {
  raw: InventoryItem;
  /** Mejor match encontrado en la búsqueda. null si no se halló. */
  match: Addon | null;
  /** Otros resultados que el usuario puede preferir. */
  alternatives: Addon[];
  /** Estado de la resolución. */
  status: "pending" | "resolving" | "resolved" | "not_found" | "error";
}

interface Props {
  path: string;
  onClose: () => void;
}

export function ImportInventoryModal({ path, onClose }: Props) {
  const [items, setItems] = useState<ResolvedItem[]>([]);
  const [parseError, setParseError] = useState<string | null>(null);
  const [resolving, setResolving] = useState(false);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [sources, setSources] = useState<SourceDescriptor[]>([]);
  const startDownload = useDownloadsStore((s) => s.start);

  useEffect(() => {
    let cancelled = false;
    api
      .listSources()
      .then((s) => {
        if (!cancelled) setSources(s);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const raw = await readFile(path);
        const parsed = parseInventory(raw, path);
        if (cancelled) return;
        const initial: ResolvedItem[] = parsed.map((p) => ({
          raw: p,
          match: null,
          alternatives: [],
          status: "pending",
        }));
        setItems(initial);
        setResolving(true);
        // Resolver de a uno para no inundar la red. Cada item tarda
        // ~300-800ms; con 30 items eso es ~10-25s. No bloqueamos
        // el render porque los items van apareciendo conforme se
        // resuelven.
        for (let i = 0; i < initial.length; i++) {
          if (cancelled) return;
          const item = initial[i];
          const query = (item.raw.icao || item.raw.rawTitle).trim();
          if (!query) {
            updateItem(i, (it) => ({ ...it, status: "not_found" }));
            continue;
          }
          updateItem(i, (it) => ({ ...it, status: "resolving" }));
          try {
            const results: Addon[] = [];
            for (const src of (await api.listSources())) {
              const r = await api.search(query, src.id);
              results.push(...r);
            }
            if (cancelled) return;
            const best = pickBest(item.raw, results);
            updateItem(i, (it) => ({
              ...it,
              match: best,
              alternatives: results.filter((a) => a.id !== best?.id).slice(0, 5),
              status: best ? "resolved" : "not_found",
            }));
            if (best) {
              setSelected((prev) => {
                const next = new Set(prev);
                next.add(i);
                return next;
              });
            }
          } catch (e) {
            updateItem(i, (it) => ({ ...it, status: "error" }));
            console.warn("import resolve item failed:", e);
          }
        }
        if (!cancelled) setResolving(false);
      } catch (e) {
        if (!cancelled) setParseError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [path]);

  const updateItem = (i: number, fn: (it: ResolvedItem) => ResolvedItem) =>
    setItems((prev) => {
      const next = [...prev];
      next[i] = fn(next[i]);
      return next;
    });

  const grouped = useMemo(() => {
    const groups = new Map<CategoryKey, number[]>();
    for (let i = 0; i < items.length; i++) {
      const cat = categorize(items[i]);
      if (!groups.has(cat)) groups.set(cat, []);
      groups.get(cat)!.push(i);
    }
    return Array.from(groups.entries()).sort(
      (a, b) => CATEGORY_ORDER.indexOf(a[0]) - CATEGORY_ORDER.indexOf(b[0]),
    );
  }, [items]);

  const toggle = (i: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });

  const enqueueAll = () => {
    let queued = 0;
    for (const i of selected) {
      const it = items[i];
      const match = it.match;
      if (!match) continue;
      // Tomamos el primer método disponible (preferencia torrent → mirror).
      const method =
        match.downloadMethods.find((m) => m.kind === "torrent") ??
        match.downloadMethods[0];
      if (!method) continue;
      startDownload({
        addonId: match.id,
        addonTitle: match.name,
        source: match.source,
        method,
        addonSimulator: match.simulator,
      });
      queued++;
    }
    onClose();
    if (queued > 0) {
      window.dispatchEvent(
        new CustomEvent("msfs-addons:toast", {
          detail: {
            kind: "info",
            message: t("import.queued_toast", { count: String(queued) }),
          },
        }),
      );
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        className="fixed inset-0 z-[55] flex items-start justify-center overflow-y-auto bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={onClose}
      >
        <motion.div
          className="relative w-full max-w-3xl rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl"
          initial={{ y: 20, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 20, opacity: 0 }}
          transition={{ duration: 0.18 }}
          onClick={(e) => e.stopPropagation()}
        >
          <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
            <div className="flex items-center gap-2">
              <FileText className="h-4 w-4 text-brand-300" />
              <div>
                <h2 className="text-sm font-semibold text-slate-100">
                  {t("import.title")}
                </h2>
                <p className="text-[11px] text-slate-500 font-mono truncate max-w-[400px]" title={path}>
                  {path}
                </p>
              </div>
            </div>
            <button
              onClick={onClose}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          {parseError && (
            <div className="m-5 flex items-start gap-2 rounded-md border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{parseError}</span>
            </div>
          )}

          {!parseError && items.length === 0 && (
            <div className="flex items-center justify-center gap-2 py-12 text-xs text-slate-400">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("import.reading")}
            </div>
          )}

          {items.length > 0 && (
            <>
              <div className="border-b border-slate-800 px-5 py-2 text-[11px] text-slate-500">
                {t("import.entries_detected", { count: String(items.length) })} ·{" "}
                <span className="text-emerald-300">
                  {t("import.resolved", {
                    count: String(items.filter((i) => i.status === "resolved").length),
                  })}
                </span>{" "}
                ·{" "}
                <span className="text-amber-300">
                  {t("import.unmatched", {
                    count: String(items.filter((i) => i.status === "not_found").length),
                  })}
                </span>
                {resolving && <span className="ml-2 text-brand-300">{t("import.resolving")}</span>}
                <span className="ml-3 text-slate-400">
                  {t("import.sources", { count: String(sources.length) })}
                </span>
              </div>
              <div className="max-h-[55vh] overflow-y-auto p-4">
                {grouped.map(([cat, idxs]) => (
                  <section key={cat} className="mb-4 last:mb-0">
                    <h3 className="mb-2 text-[10px] font-semibold uppercase tracking-wider text-slate-400">
                      {t(cat)} ({idxs.length})
                    </h3>
                    <ul className="space-y-1.5">
                      {idxs.map((i) => {
                        const it = items[i];
                        return (
                          <li
                            key={i}
                            className={`flex items-start gap-2 rounded-md border px-3 py-2 ${
                              selected.has(i)
                                ? "border-brand-500/50 bg-brand-500/5"
                                : "border-slate-800 bg-slate-900/40"
                            }`}
                          >
                            <input
                              type="checkbox"
                              checked={selected.has(i)}
                              disabled={!it.match}
                              onChange={() => toggle(i)}
                              className="mt-1 accent-brand-500 disabled:opacity-30"
                            />
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-2">
                                {it.raw.icao && (
                                  <span className="font-mono text-[11px] font-semibold text-brand-300">
                                    {it.raw.icao}
                                  </span>
                                )}
                                <span className="truncate text-xs text-slate-200">
                                  {it.raw.rawTitle}
                                </span>
                              </div>
                              <div className="mt-0.5 text-[10px] text-slate-500">
                                {it.raw.developer && (
                                  <span className="mr-2">{it.raw.developer}</span>
                                )}
                                {statusLabel(it)}
                              </div>
                            </div>
                          </li>
                        );
                      })}
                    </ul>
                  </section>
                ))}
              </div>
              <footer className="flex items-center justify-between gap-2 border-t border-slate-800 bg-slate-900/40 px-5 py-3">
                <div className="text-[11px] text-slate-400">
                  <CheckCircle2 className="mr-1 inline h-3 w-3 text-emerald-300" />
                  {t("import.selected", {
                    selected: String(selected.size),
                    total: String(items.length),
                  })}
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={onClose}
                    className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700"
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    onClick={enqueueAll}
                    disabled={selected.size === 0 || resolving}
                    className="inline-flex items-center gap-1 rounded-md bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-40"
                  >
                    <Download className="h-3.5 w-3.5" />
                    {t("import.download_selected")}
                  </button>
                </div>
              </footer>
            </>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}

function statusLabel(it: ResolvedItem): React.ReactNode {
  switch (it.status) {
    case "resolving":
      return (
        <span className="inline-flex items-center gap-1 text-brand-300">
          <Loader2 className="h-2.5 w-2.5 animate-spin" /> {t("import.status.searching")}
        </span>
      );
    case "resolved":
      return (
        <span className="text-emerald-300">
          <SearchIcon className="mr-0.5 inline h-2.5 w-2.5" />
          {it.match?.name} ({it.match?.source})
        </span>
      );
    case "not_found":
      return <span className="text-slate-500">{t("import.status.no_match")}</span>;
    case "error":
      return <span className="text-rose-300">{t("import.status.error")}</span>;
    default:
      return null;
  }
}

const CATEGORY_KEYS = [
  "addons.section.airports",
  "addons.section.aircraft",
  "addons.section.liveries",
  "addons.section.misc",
] as const;
type CategoryKey = (typeof CATEGORY_KEYS)[number];
const CATEGORY_ORDER: CategoryKey[] = [...CATEGORY_KEYS];

function categorize(it: ResolvedItem): CategoryKey {
  const title = (it.match?.name || it.raw.rawTitle).toLowerCase();
  if (it.raw.icao || it.match?.icao) return "addons.section.airports";
  if (/livery|paint|repaint/.test(title)) return "addons.section.liveries";
  if (/[ab]\d{3}|73[6-9]|74[478]|77\d|78\d|crj|atr|md-|tbm|c\d{3}/.test(title)) {
    return "addons.section.aircraft";
  }
  return "addons.section.misc";
}

async function readFile(path: string): Promise<string> {
  // (v1.1.4) Lee del backend Rust via comando IPC para no añadir la
  // dep `@tauri-apps/plugin-fs` (que pesa ~50KB de JS por sólo esto).
  return await api.readTextFile(path);
}

function parseInventory(content: string, path: string): InventoryItem[] {
  const ext = path.toLowerCase().split(".").pop() || "";
  if (ext === "json") return parseJson(content);
  if (ext === "csv") return parseCsv(content);
  return parseTxt(content);
}

function parseJson(content: string): InventoryItem[] {
  const arr = JSON.parse(content);
  if (!Array.isArray(arr)) throw new Error(t("import.error.json_not_array"));
  return arr.map((row): InventoryItem => ({
    rawTitle: String(row.title || row.name || row.folderName || ""),
    icao: row.icao ? String(row.icao).toUpperCase() : undefined,
    developer: row.creator || row.developer || undefined,
    version: row.packageVersion || row.version || undefined,
  })).filter((r) => r.rawTitle);
}

function parseCsv(content: string): InventoryItem[] {
  const lines = content.split(/\r?\n/).filter((l) => l.trim());
  if (lines.length < 2) return [];
  const header = lines[0].split(",").map((s) => s.trim().toLowerCase());
  const idxTitle = header.findIndex((h) => h === "title" || h === "name");
  const idxIcao = header.findIndex((h) => h === "icao");
  const idxCreator = header.findIndex((h) => h === "creator" || h === "developer");
  const idxVersion = header.findIndex((h) => h === "version" || h === "packageversion");
  const out: InventoryItem[] = [];
  for (let i = 1; i < lines.length; i++) {
    const cells = splitCsv(lines[i]);
    if (cells.length === 0) continue;
    const title = idxTitle >= 0 ? cells[idxTitle] : cells[0];
    if (!title) continue;
    out.push({
      rawTitle: title,
      icao: idxIcao >= 0 && cells[idxIcao] ? cells[idxIcao].toUpperCase() : undefined,
      developer: idxCreator >= 0 ? cells[idxCreator] : undefined,
      version: idxVersion >= 0 ? cells[idxVersion] : undefined,
    });
  }
  return out;
}

function splitCsv(line: string): string[] {
  const out: string[] = [];
  let cur = "";
  let inQ = false;
  for (const c of line) {
    if (c === '"') {
      inQ = !inQ;
    } else if (c === "," && !inQ) {
      out.push(cur);
      cur = "";
    } else {
      cur += c;
    }
  }
  out.push(cur);
  return out.map((s) => s.trim());
}

function parseTxt(content: string): InventoryItem[] {
  return content
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"))
    .map((line): InventoryItem => {
      // Buscar un ICAO de 4 letras al inicio.
      const m = line.match(/^([A-Z]{4})\b/);
      return {
        rawTitle: line,
        icao: m ? m[1] : undefined,
      };
    });
}

function pickBest(raw: InventoryItem, results: Addon[]): Addon | null {
  if (results.length === 0) return null;
  // Prioridad: matching ICAO + creator → matching ICAO → matching title.
  const rawDev = (raw.developer || "").toLowerCase();
  const rawIcao = (raw.icao || "").toUpperCase();
  let best: Addon | null = null;
  let bestScore = -1;
  for (const r of results) {
    let score = 0;
    if (rawIcao && r.icao?.toUpperCase() === rawIcao) score += 4;
    if (rawDev && r.developer?.toLowerCase().includes(rawDev)) score += 2;
    if (
      raw.rawTitle &&
      r.name.toLowerCase().includes(raw.rawTitle.toLowerCase())
    ) {
      score += 1;
    }
    if (score > bestScore) {
      best = r;
      bestScore = score;
    }
  }
  return best;
}
