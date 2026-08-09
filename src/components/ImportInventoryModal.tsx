import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  ExternalLink,
  FileText,
  Loader2,
  Search as SearchIcon,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import type { Addon, SourceDescriptor } from "../lib/types";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { t } from "../lib/i18n";
import {
  flightsimSearchUrl,
  paywareVendorFor,
  type PaywareVendor,
} from "../lib/addonOrigin";

/**
 * (v7.5) Import de inventario + RE-DESCARGA SELECTIVA. El usuario, tras
 * formatear el PC, importa el JSON exportado y por cada addon elige DÓNDE
 * buscarlo (la app detecta y sugiere la fuente):
 *
 *   · Fuente de catálogo (SceneryAddons/Simplaza/Skybound) → baja por la app.
 *   · flightsim.to → abre el webview embebido (auto o manual) → modal install.
 *   · Payware (PMDG/Fenix/iniBuilds/…) → abre la página OFICIAL del dev.
 *   · No descargar → lo salta.
 *
 * Al "Finalizar" procesa los addons 1×1 (secuencial): los torrent se encolan
 * en background; los que abren webview se hacen de a uno, esperando a que el
 * modal de instalación de cada uno termine (evento `msfs-addons:drop-flow-done`
 * que emite `DragDropOverlay`) antes de pasar al siguiente.
 */

interface InventoryItem {
  rawTitle: string;
  icao?: string;
  developer?: string;
  version?: string;
  /** (v7.5) Origen REAL del export: "flightsimto" | "<catalog>" | "unknown". */
  origin?: string;
  /** URL directa para re-bajar (link de flightsim.to o page_url del catálogo). */
  originUrl?: string;
}

/** Ruta elegida para un item: id de fuente de catálogo, o pseudo-fuentes. */
type Route = string; // <sourceId> | "flightsimto" | "payware" | "skip"

interface ResolvedItem {
  raw: InventoryItem;
  match: Addon | null;
  alternatives: Addon[];
  status: "pending" | "resolving" | "resolved" | "not_found" | "error";
  /** Vendor payware detectado por el creator (si aplica). */
  payware: PaywareVendor | null;
  /** Ruta elegida por el usuario (o el default calculado). */
  route: Route;
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
  const [restoring, setRestoring] = useState(false);
  const [restoreAt, setRestoreAt] = useState<{ i: number; total: number } | null>(null);
  const startDownload = useDownloadsStore((s) => s.start);
  const abortRef = useRef(false);

  useEffect(() => {
    abortRef.current = false;
    return () => {
      abortRef.current = true;
    };
  }, []);

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
          payware: paywareVendorFor(p.developer, p.rawTitle),
          route: initialRoute(p),
        }));
        setItems(initial);
        setResolving(true);
        const srcList = await api.listSources();
        for (let i = 0; i < initial.length; i++) {
          if (cancelled) return;
          const item = initial[i];
          const query = (item.raw.icao || item.raw.rawTitle).trim();
          if (!query) {
            updateItem(i, (it) => ({
              ...it,
              status: "not_found",
              route: defaultRoute(it, []),
            }));
            continue;
          }
          updateItem(i, (it) => ({ ...it, status: "resolving" }));
          try {
            const results: Addon[] = [];
            for (const src of srcList) {
              const r = await api.search(query, src.id);
              results.push(...r);
            }
            if (cancelled) return;
            const best = pickBest(item.raw, results);
            const alts = results.filter((a) => a.id !== best?.id).slice(0, 5);
            updateItem(i, (it) => {
              const next: ResolvedItem = {
                ...it,
                match: best,
                alternatives: alts,
                status: best ? "resolved" : "not_found",
              };
              next.route = defaultRoute(next, best ? [best, ...alts] : alts);
              return next;
            });
            // Auto-seleccionar todo lo que tenga una ruta útil (no "skip").
            setSelected((prev) => {
              const next = new Set(prev);
              next.add(i);
              return next;
            });
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

  const setRoute = (i: number, route: Route) =>
    updateItem(i, (it) => ({ ...it, route }));

  /** Fuentes candidatas para un item (para el <select>). */
  const routeOptions = (it: ResolvedItem): { value: Route; label: string }[] => {
    const opts: { value: Route; label: string }[] = [];
    const seen = new Set<string>();
    for (const a of [it.match, ...it.alternatives].filter(Boolean) as Addon[]) {
      if (seen.has(a.source)) continue;
      seen.add(a.source);
      opts.push({ value: a.source, label: sourceLabel(a.source, sources) });
    }
    opts.push({ value: "flightsimto", label: "flightsim.to" });
    if (it.payware) opts.push({ value: "payware", label: `Payware — ${it.payware.name}` });
    opts.push({ value: "skip", label: t("import.route_skip") });
    return opts;
  };

  /** Espera a que un addon termine su flujo: instalado (`drop-flow-done`),
   *  saltado a mano (`restore-skip`), webview CERRADO sin bajar (si conocemos
   *  su label), o timeout de seguridad. Esto evita que "restaurando N/M" se
   *  quede trabado cuando cerrás el webview sin descargar. */
  const awaitItemDone = (webviewLabel?: string): Promise<void> =>
    new Promise((resolve) => {
      let done = false;
      let seen = false;
      const finish = () => {
        if (done) return;
        done = true;
        window.removeEventListener("msfs-addons:drop-flow-done", finish);
        window.removeEventListener("msfs-addons:restore-skip", finish);
        window.clearInterval(poll);
        window.clearTimeout(timer);
        resolve();
      };
      window.addEventListener("msfs-addons:drop-flow-done", finish);
      window.addEventListener("msfs-addons:restore-skip", finish);
      const poll = window.setInterval(async () => {
        if (!webviewLabel || abortRef.current) {
          if (abortRef.current) finish();
          return;
        }
        try {
          const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
          const w = await WebviewWindow.getByLabel(webviewLabel);
          if (w) seen = true;
          else if (seen) finish();
        } catch {
          /* demo/browser: sin webview nativo */
        }
      }, 900);
      const timer = window.setTimeout(finish, 10 * 60 * 1000);
    });

  const finalize = async () => {
    const order = [...selected].sort((a, b) => a - b);
    if (order.length === 0) return;
    setRestoring(true);
    let idx = 0;
    for (const i of order) {
      if (abortRef.current) break;
      idx++;
      setRestoreAt({ i: idx, total: order.length });
      const it = items[i];
      const route = it.route;
      if (route === "skip") continue;

      if (route === "payware") {
        const pw = it.payware ?? paywareVendorFor(it.raw.developer, it.raw.rawTitle);
        if (pw) {
          try {
            await api.openExternal(pw.url);
          } catch (e) {
            console.warn("openExternal payware failed:", e);
          }
        }
        // Payware = descarga/compra manual en su sitio; no esperamos install.
        continue;
      }

      if (route === "flightsimto") {
        try {
          // Con el link directo del export abrimos el archivo EXACTO en
          // flightsim.to (countdown + auto-download). Sin él, la búsqueda.
          const url =
            it.raw.origin === "flightsimto" && it.raw.originUrl
              ? it.raw.originUrl
              : flightsimSearchUrl(it.raw.rawTitle, it.raw.developer);
          await api.openLiveryBrowser(url);
          // Avanza al instalar, al saltar, o si CERRÁS el webview sin bajar.
          await awaitItemDone("livery-browser");
        } catch (e) {
          console.warn("flightsim.to route failed:", e);
        }
        continue;
      }

      // Fuente de catálogo: buscamos el Addon de esa fuente.
      const addon = [it.match, ...it.alternatives]
        .filter(Boolean)
        .find((a) => (a as Addon).source === route) as Addon | undefined;
      if (!addon || addon.downloadMethods.length === 0) continue;
      const method =
        addon.downloadMethods.find((m) => m.kind === "torrent") ??
        addon.downloadMethods[0];
      try {
        await startDownload({
          addonId: addon.id,
          addonTitle: addon.name,
          source: addon.source,
          method,
          addonSimulator: addon.simulator,
          allMethods: addon.downloadMethods,
        });
        // Mirror/direct abren webview + modal → secuencial (esperamos).
        // Torrent baja en background → seguimos sin esperar.
        if (method.kind !== "torrent") {
          await awaitItemDone();
        }
      } catch (e) {
        console.warn("catalog route failed:", e);
      }
    }
    setRestoring(false);
    setRestoreAt(null);
    if (!abortRef.current) onClose();
  };

  const eligible = useMemo(
    () => [...selected].filter((i) => items[i] && items[i].route !== "skip").length,
    [selected, items],
  );

  return (
    <AnimatePresence>
      <motion.div
        className="fixed inset-0 z-[55] flex items-start justify-center overflow-y-auto bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={() => {
          if (!restoring) onClose();
        }}
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
              disabled={restoring}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-30"
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
                        const opts = routeOptions(it);
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
                              onChange={() => toggle(i)}
                              className="mt-1 accent-brand-500"
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
                            {/* Selector de FUENTE por-addon. */}
                            <div className="flex shrink-0 items-center gap-1">
                              <span className="text-[9px] uppercase tracking-wide text-slate-600">
                                {t("import.route_label")}
                              </span>
                              <select
                                value={it.route}
                                onChange={(e) => setRoute(i, e.target.value)}
                                disabled={restoring}
                                className="max-w-[150px] rounded border border-slate-700 bg-slate-900 px-1.5 py-1 text-[11px] text-slate-200 focus:border-brand-500 focus:outline-none disabled:opacity-40"
                              >
                                {opts.map((o) => (
                                  <option key={o.value} value={o.value}>
                                    {o.label}
                                  </option>
                                ))}
                              </select>
                              {it.route === "payware" && (
                                <ExternalLink className="h-3 w-3 text-amber-400" />
                              )}
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
                  {restoreAt ? (
                    <span className="inline-flex items-center gap-1 text-brand-300">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      {t("import.restoring", {
                        i: String(restoreAt.i),
                        total: String(restoreAt.total),
                      })}
                    </span>
                  ) : (
                    <>
                      <CheckCircle2 className="mr-1 inline h-3 w-3 text-emerald-300" />
                      {t("import.selected", {
                        selected: String(eligible),
                        total: String(items.length),
                      })}
                    </>
                  )}
                </div>
                <div className="flex gap-2">
                  {restoring ? (
                    <button
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("msfs-addons:restore-skip"),
                        )
                      }
                      className="inline-flex items-center gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-1.5 text-xs font-semibold text-amber-200 hover:bg-amber-500/20"
                    >
                      {t("import.skip_current")}
                    </button>
                  ) : (
                    <>
                      <button
                        onClick={onClose}
                        className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700"
                      >
                        {t("common.cancel")}
                      </button>
                      <button
                        onClick={finalize}
                        disabled={eligible === 0 || resolving}
                        className="inline-flex items-center gap-1 rounded-md bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-40"
                      >
                        <Download className="h-3.5 w-3.5" />
                        {t("import.finalize")}
                      </button>
                    </>
                  )}
                </div>
              </footer>
            </>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}

/** Ruta por defecto tras resolver: ORIGEN REAL > payware > match > flightsim.to. */
function defaultRoute(it: ResolvedItem, candidates: Addon[]): Route {
  const origin = it.raw.origin;
  if (origin === "flightsimto") return "flightsimto";
  if (it.payware) return "payware";
  if (origin && origin !== "unknown" && candidates.some((a) => a.source === origin)) {
    return origin;
  }
  if (it.match) return it.match.source;
  if (candidates.length > 0) return candidates[0].source;
  return "flightsimto";
}

/** Ruta inicial (antes de la búsqueda) según el origen exportado. */
function initialRoute(p: InventoryItem): Route {
  if (p.origin === "flightsimto") return "flightsimto";
  if (paywareVendorFor(p.developer, p.rawTitle)) return "payware";
  return "skip";
}

function sourceLabel(id: string, sources: SourceDescriptor[]): string {
  const s = sources.find((x) => x.id === id);
  return s?.name ?? id;
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
      return it.payware ? (
        <span className="text-amber-300">Payware — {it.payware.name}</span>
      ) : (
        <span className="text-slate-500">{t("import.status.no_match")}</span>
      );
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
  return arr
    .map((row): InventoryItem => ({
      rawTitle: String(row.title || row.name || row.folderName || row.folder_name || ""),
      icao: row.icao ? String(row.icao).toUpperCase() : undefined,
      developer: row.creator || row.developer || undefined,
      version: row.packageVersion || row.package_version || row.version || undefined,
      origin: row.origin ? String(row.origin) : undefined,
      originUrl: row.origin_url || row.originUrl || undefined,
    }))
    .filter((r) => r.rawTitle);
}

function parseCsv(content: string): InventoryItem[] {
  const lines = content.split(/\r?\n/).filter((l) => l.trim());
  if (lines.length < 2) return [];
  const header = lines[0].split(",").map((s) => s.trim().toLowerCase());
  const idxTitle = header.findIndex((h) => h === "title" || h === "name");
  const idxIcao = header.findIndex((h) => h === "icao");
  const idxCreator = header.findIndex((h) => h === "creator" || h === "developer");
  const idxVersion = header.findIndex((h) => h === "version" || h === "packageversion");
  const idxOrigin = header.findIndex((h) => h === "origin");
  const idxOriginUrl = header.findIndex((h) => h === "origin_url" || h === "originurl");
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
      origin: idxOrigin >= 0 && cells[idxOrigin] ? cells[idxOrigin] : undefined,
      originUrl: idxOriginUrl >= 0 && cells[idxOriginUrl] ? cells[idxOriginUrl] : undefined,
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
      const m = line.match(/^([A-Z]{4})\b/);
      return {
        rawTitle: line,
        icao: m ? m[1] : undefined,
      };
    });
}

function pickBest(raw: InventoryItem, results: Addon[]): Addon | null {
  if (results.length === 0) return null;
  const rawDev = (raw.developer || "").toLowerCase();
  const rawIcao = (raw.icao || "").toUpperCase();
  const rawOrigin = raw.origin;
  let best: Addon | null = null;
  let bestScore = 0; // (fix) exigir score > 0 — antes aceptaba cualquier hit.
  for (const r of results) {
    let score = 0;
    if (rawOrigin && rawOrigin !== "unknown" && r.source === rawOrigin) score += 3;
    if (rawIcao && r.icao?.toUpperCase() === rawIcao) score += 4;
    if (rawDev && r.developer?.toLowerCase().includes(rawDev)) score += 2;
    if (raw.rawTitle && r.name.toLowerCase().includes(raw.rawTitle.toLowerCase())) {
      score += 1;
    }
    if (score > bestScore) {
      best = r;
      bestScore = score;
    }
  }
  return best;
}
