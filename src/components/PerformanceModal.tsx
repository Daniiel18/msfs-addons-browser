import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import {
  CheckCheck,
  Gauge,
  Info,
  Loader2,
  MapPin,
  RefreshCw,
  RotateCcw,
  Sparkles,
  X,
  Zap,
} from "lucide-react";
import type { CommunityPackage, PerfConfig, PerfOption } from "../lib/types";
import { api } from "../lib/tauri";
import { useToastStore } from "../stores/useToastStore";
import { usePerfStore } from "../stores/usePerfStore";
import { useThumbnail } from "../lib/thumbnails";
import { looksLikePlaceholderTitle } from "../lib/packageType";
import { ToggleSwitch } from "./AddonToggle";
import { t } from "../lib/i18n";

/**
 * (v5.0.0) **Modal de Rendimiento (FPS)** de un aeropuerto.
 *
 * Lee `config/simfleet_perf.json` (que el backend generó al instalar el
 * escenario, escaneando los `.bgl` opcionales) y pinta un toggle por
 * cada grupo de objetos pesados desactivables — autos estáticos,
 * pasajeros 3D, GSE, agentes animados… Cada fila muestra qué hace y un
 * estimado de FPS ahorrado.
 *
 * El switch representa la PRESENCIA de los objetos:
 *   · ON  (verde) = objetos presentes (escenario completo).
 *   · OFF (apagado) = `.bgl` → `.bgl.off`, MSFS los ignora → ganas FPS.
 *
 * El toggle renombra los archivos físicamente en tiempo real (atómico,
 * con rollback si MSFS tiene el escenario abierto).
 *
 * Botón "Actualizar desde SceneryAddons": baja la nota "Optional
 * Configuration" del dev para enriquecer etiquetas + estimados de FPS.
 */
export function PerformanceModal({
  pkg,
  onClose,
}: {
  pkg: CommunityPackage;
  onClose: () => void;
}) {
  const pushToast = useToastStore((s) => s.push);
  const markOptimizable = usePerfStore((s) => s.markOptimizable);
  const thumb = useThumbnail(pkg.folderName, looksLikePlaceholderTitle(pkg.title));
  const [config, setConfig] = useState<PerfConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [enriching, setEnriching] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [bulkBusy, setBulkBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    (async () => {
      setLoading(true);
      try {
        const cfg = await api.perfReadConfig(
          pkg.installPath,
          pkg.folderName,
          pkg.icao,
        );
        if (alive) {
          setConfig(cfg);
          // Badge sólo si viene de la página (no del escaneo local).
          if (cfg && cfg.options.length > 0 && cfg.source !== "local-scan")
            markOptimizable(pkg.folderName);
        }
      } catch (e) {
        if (alive)
          pushToast({ kind: "error", title: t("perf.load_error"), message: String(e) });
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [pkg.installPath, pkg.folderName, pkg.icao, pushToast]);

  // Aplica/revierte una opción multiestado (renombra sus .bgl según la
  // dirección de cada archivo). `applied` = la optimización del dev está
  // en efecto.
  const toggle = async (opt: PerfOption) => {
    if (busyId) return;
    setBusyId(opt.id);
    try {
      const res = await api.perfToggleOption(pkg.installPath, opt.id, !opt.applied);
      setConfig((prev) =>
        prev
          ? {
              ...prev,
              options: prev.options.map((o) =>
                o.id === opt.id ? res.option : o,
              ),
            }
          : prev,
      );
      pushToast({
        kind: "success",
        title: res.option.applied
          ? t("perf.applied", { label: optLabel(res.option) })
          : t("perf.reverted", { label: optLabel(res.option) }),
        message: t("perf.renamed", { n: String(res.renamed) }),
        ttlMs: 2600,
      });
    } catch (e) {
      // El backend ya mapea "os error 32" a un mensaje claro (MSFS abierto).
      pushToast({ kind: "error", title: t("perf.toggle_error"), message: String(e) });
    } finally {
      setBusyId(null);
    }
  };

  // (v5.0.0) Aplica (apply=true) o revierte (false) TODAS las opciones
  // que no estén ya en ese estado. Secuencial, con rollback por opción.
  const setAll = async (apply: boolean) => {
    if (bulkBusy || busyId) return;
    const targets = (config?.options ?? []).filter((o) => o.applied !== apply);
    if (targets.length === 0) return;
    setBulkBusy(true);
    let renamed = 0;
    try {
      for (const o of targets) {
        try {
          const res = await api.perfToggleOption(pkg.installPath, o.id, apply);
          renamed += res.renamed;
          setConfig((prev) =>
            prev
              ? {
                  ...prev,
                  options: prev.options.map((x) =>
                    x.id === o.id ? res.option : x,
                  ),
                }
              : prev,
          );
        } catch (e) {
          // p. ej. MSFS con el escenario abierto → paramos en seco.
          pushToast({ kind: "error", title: t("perf.toggle_error"), message: String(e) });
          break;
        }
      }
      pushToast({
        kind: "success",
        title: apply ? t("perf.applied_all") : t("perf.reverted_all"),
        message: t("perf.renamed", { n: String(renamed) }),
        ttlMs: 2600,
      });
    } finally {
      setBulkBusy(false);
    }
  };

  // Baja la nota del DEV instalado desde SceneryAddons (el backend
  // resuelve la página correcta por creator+folder).
  const enrich = async () => {
    if (!pkg.icao) return;
    setEnriching(true);
    try {
      const cfg = await api.perfEnrichFromSource(
        pkg.installPath,
        pkg.folderName,
        pkg.icao,
        pkg.creator,
      );
      setConfig(cfg);
      const fromNote = !!cfg && cfg.options.some((o) => o.fromNote);
      if (cfg && cfg.options.length > 0 && cfg.source !== "local-scan")
        markOptimizable(pkg.folderName);
      pushToast(
        fromNote
          ? { kind: "success", title: t("perf.enriched"), ttlMs: 2600 }
          : { kind: "info", title: t("perf.no_source") },
      );
    } catch (e) {
      pushToast({ kind: "error", title: t("perf.enrich_error"), message: String(e) });
    } finally {
      setEnriching(false);
    }
  };

  // (v5.0.0) Localización: las opciones de la NOTA usan la etiqueta del
  // dev (inglés, de la página); las del escaneo LOCAL se localizan por
  // categoría con i18n. La descripción siempre se localiza por categoría.
  const optLabel = (o: PerfOption) => {
    if (o.fromNote && o.label) return o.label;
    const k = `perf.cat.${o.category}.label`;
    const v = t(k);
    return v === k ? o.label || o.category : v;
  };
  // Un swap (p. ej. High→Medium) tiene ops en ambas direcciones.
  const isSwap = (o: PerfOption) =>
    o.ops.some((op) => op.activeWhenApplied) &&
    o.ops.some((op) => !op.activeWhenApplied);
  const optDesc = (o: PerfOption) => {
    if (isSwap(o)) return t("perf.swap_desc");
    const k = `perf.cat.${o.category}.desc`;
    const v = t(k);
    return v === k ? o.description : v;
  };

  const options = config?.options ?? [];
  // Optimizaciones actualmente aplicadas (ahorrando FPS).
  const savedCount = options.filter((o) => o.applied).length;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.97, y: 8 }}
        transition={{ duration: 0.18 }}
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[min(680px,calc(100vh-3rem))] w-[min(560px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
      >
        {/* (v5.0.0) Vista previa del escenario arriba del contenedor. */}
        <div className="relative h-28 w-full shrink-0 overflow-hidden bg-gradient-to-br from-slate-800 to-slate-950">
          {thumb ? (
            <img
              src={thumb}
              alt=""
              className="h-full w-full object-cover"
              onError={(e) => {
                (e.currentTarget as HTMLImageElement).style.display = "none";
              }}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-slate-700">
              <MapPin className="h-10 w-10" />
            </div>
          )}
          <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950 via-slate-950/30 to-transparent" />
        </div>

        <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-emerald-500/15 ring-1 ring-emerald-500/40">
              <Gauge className="h-4 w-4 text-emerald-300" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-slate-100">
                {t("perf.title")}
              </h3>
              <p className="text-[11px] text-slate-500">
                {pkg.airportName ?? pkg.title}
                {pkg.icao && <span className="font-mono"> · {pkg.icao}</span>}
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

        {/* Nota explicativa del comportamiento del switch. */}
        <div className="flex items-start gap-2 border-b border-slate-800 bg-slate-900/40 px-5 py-2.5 text-[11px] text-slate-400">
          <Info className="mt-0.5 h-3.5 w-3.5 shrink-0 text-sky-400" />
          <p>{t("perf.help")}</p>
        </div>

        {/* (v5.0.0) Acciones masivas + contador de aplicadas. */}
        {!loading && options.length > 0 && (
          <div className="flex items-center justify-between gap-2 border-b border-slate-800 px-5 py-2">
            <span className="text-[11px] font-medium text-slate-300">
              {t("perf.applied_count", {
                n: String(savedCount),
                total: String(options.length),
              })}
            </span>
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => void setAll(true)}
                disabled={bulkBusy || !!busyId || savedCount === options.length}
                className="inline-flex items-center gap-1 rounded-md border border-emerald-500/40 bg-emerald-500/10 px-2 py-1 text-[11px] font-medium text-emerald-200 hover:bg-emerald-500/20 disabled:opacity-40"
              >
                {bulkBusy ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <CheckCheck className="h-3 w-3" />
                )}
                {t("perf.mark_all")}
              </button>
              <button
                onClick={() => void setAll(false)}
                disabled={bulkBusy || !!busyId || savedCount === 0}
                className="inline-flex items-center gap-1 rounded-md border border-slate-700 bg-slate-800/60 px-2 py-1 text-[11px] font-medium text-slate-300 hover:bg-slate-800 disabled:opacity-40"
              >
                <RotateCcw className="h-3 w-3" />
                {t("perf.unmark_all")}
              </button>
            </div>
          </div>
        )}

        <div className="perf-scroll min-h-0 flex-1 overflow-y-scroll px-5 py-3">
          {loading ? (
            <div className="flex items-center justify-center py-12 text-slate-500">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : options.length === 0 ? (
            <div className="flex flex-col items-center gap-2 py-10 text-center">
              <Sparkles className="h-7 w-7 text-slate-600" />
              <p className="text-sm font-medium text-slate-300">
                {t("perf.empty.title")}
              </p>
              <p className="max-w-xs text-[12px] text-slate-500">
                {t("perf.empty.body")}
              </p>
            </div>
          ) : (
            <ul className="space-y-2">
              {options.map((opt) => (
                <li
                  key={opt.id}
                  className={`rounded-xl border px-3.5 py-3 transition-colors ${
                    opt.applied
                      ? "border-emerald-600/40 bg-emerald-950/20"
                      : "border-slate-800 bg-slate-900/40"
                  }`}
                >
                  <div className="flex items-start gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <p className="truncate text-[13px] font-semibold text-slate-100">
                          {optLabel(opt)}
                        </p>
                        <span className="inline-flex shrink-0 items-center gap-0.5 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-bold text-amber-300 ring-1 ring-amber-500/30">
                          <Zap className="h-2.5 w-2.5" />
                          {opt.fpsHint}
                        </span>
                      </div>
                      <p className="mt-1 text-[11px] leading-relaxed text-slate-400">
                        {optDesc(opt)}
                      </p>
                      <p className="mt-1 text-[10px] text-slate-600">
                        {opt.ops.length === 1
                          ? t("perf.one_file")
                          : t("perf.n_files", { n: String(opt.ops.length) })}
                        {opt.applied && (
                          <span className="ml-1 font-semibold text-emerald-400">
                            · {t("perf.applied_now")}
                          </span>
                        )}
                      </p>
                    </div>
                    <div className="flex shrink-0 flex-col items-center gap-1 pt-0.5">
                      <ToggleSwitch
                        on={opt.applied}
                        busy={busyId === opt.id}
                        onToggle={() => void toggle(opt)}
                      />
                      <span className="text-[9px] uppercase tracking-wide text-slate-600">
                        {opt.applied ? t("perf.on") : t("perf.off")}
                      </span>
                    </div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>

        <footer className="flex items-center justify-between gap-2 border-t border-slate-800 px-5 py-2.5">
          <span className="text-[11px] text-slate-500">
            {savedCount > 0
              ? t("perf.footer_saving", { n: String(savedCount) })
              : t("perf.footer_full")}
          </span>
          <button
            onClick={() => void enrich()}
            disabled={enriching || !pkg.icao}
            title={t("perf.enrich_hint")}
            className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-800/60 px-2.5 py-1.5 text-[11px] font-medium text-slate-200 hover:border-brand-500/40 hover:bg-slate-800 disabled:opacity-50"
          >
            {enriching ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <RefreshCw className="h-3 w-3" />
            )}
            {t("perf.enrich")}
          </button>
        </footer>
      </motion.div>
    </div>
  );
}
