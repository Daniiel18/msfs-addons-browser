import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertTriangle,
  CheckCircle2,
  ExternalLink,
  Info,
  Loader2,
  Plane,
  RefreshCw,
  Rocket,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import { t, getActiveLocale } from "../lib/i18n";
import { type PreflightCheck } from "../lib/preflight";
import { usePreflight } from "../lib/usePreflight";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAiracUpdateStore } from "../stores/useAiracUpdateStore";
import { useNewSceneryStore } from "../stores/useNewSceneryStore";
import { useAppStore } from "../stores/useAppStore";
import { useToastStore } from "../stores/useToastStore";
import { usePreflightStore } from "../stores/usePreflightStore";

interface Props {
  onClose: () => void;
}

/**
 * (v6.2.26 / R3) Modal "¿Listo para volar?".
 *
 * Dos secciones:
 *   1. Checklist pre-vuelo — cruza el próximo vuelo (último OFP de
 *      SimBrief o el origen/destino en vivo del sim) con el escenario de
 *      salida/llegada instalado, su estado activo/update, y el AIRAC.
 *      Cada punto accionable trae su botón (activar / buscar / AIRAC).
 *   2. Radar de escenarios nuevos del catálogo (useNewSceneryStore).
 *
 * Toda la lógica de checks vive en `lib/preflight.ts` (pura, testeable).
 */
export function PreflightModal({ onClose }: Props) {
  const setEnabled = useCommunityStore((s) => s.setEnabled);
  const airacOpenUpdater = useAiracUpdateStore((s) => s.openUpdater);
  const triggerSearch = useAppStore((s) => s.triggerSearch);
  const pushToast = useToastStore((s) => s.push);
  const setBypass = usePreflightStore((s) => s.setBypass);

  const fresh = useNewSceneryStore((s) => s.fresh);
  const radarLoading = useNewSceneryStore((s) => s.loading);
  const acknowledgeAll = useNewSceneryStore((s) => s.acknowledgeAll);
  const dismissScenery = useNewSceneryStore((s) => s.dismiss);

  const [busyId, setBusyId] = useState<string | null>(null);

  const {
    result: { route, checks, issues, ready },
  } = usePreflight();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const runAction = async (check: PreflightCheck) => {
    const action = check.action;
    if (!action) return;
    if (action.kind === "search") {
      onClose();
      void triggerSearch(action.icao);
    } else if (action.kind === "airac") {
      void airacOpenUpdater();
    } else if (action.kind === "gsx") {
      setBusyId(check.id);
      try {
        const profiles = await api.gsxLookup(action.icao);
        if (profiles.length > 0) {
          await api.openExternal(profiles[0].link);
        } else {
          pushToast({ kind: "info", title: t("preflight.gsx_none", { icao: action.icao }) });
        }
      } catch (e) {
        pushToast({ kind: "error", title: t("preflight.gsx_err"), message: String(e) });
      } finally {
        setBusyId(null);
      }
    } else if (action.kind === "gsx-update") {
      // (v6.2.44) Perfil GSX con update → abre el link del perfil en
      // flightsim.to para re-descargar la versión nueva.
      void api.openExternal(action.link);
    } else if (action.kind === "enable") {
      setBusyId(check.id);
      try {
        await setEnabled(action.folderName, true);
        pushToast({
          kind: "success",
          title: t("preflight.enabled_ok"),
          ttlMs: 2200,
        });
      } catch (e) {
        pushToast({ kind: "error", title: t("preflight.enable_err"), message: String(e) });
      } finally {
        setBusyId(null);
      }
    }
  };

  const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
  const fmtDate = (iso: string | null | undefined) => {
    if (!iso) return "";
    const d = new Date(iso);
    if (isNaN(d.getTime())) return "";
    return d.toLocaleDateString(locale, {
      day: "2-digit",
      month: "short",
      year: "numeric",
    });
  };

  const openScenery = (icao: string | null, title: string) => {
    onClose();
    void triggerSearch(icao && icao.length === 4 ? icao : title, "sceneryaddons");
  };

  return (
    <AnimatePresence>
      <motion.div
        key="preflight-backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
        className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      >
        <motion.div
          key="preflight-modal"
          initial={{ opacity: 0, scale: 0.96, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          className="relative flex max-h-[calc(100vh-3rem)] w-[min(640px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          {/* Header con veredicto */}
          <header
            className={`flex items-center gap-3 border-b border-slate-800 px-5 py-3.5 ${
              ready
                ? "bg-emerald-500/10"
                : issues > 0
                  ? "bg-amber-500/10"
                  : "bg-slate-900/60"
            }`}
          >
            <div
              className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${
                ready
                  ? "bg-emerald-500/20 text-emerald-300"
                  : "bg-amber-500/20 text-amber-300"
              }`}
            >
              <Rocket className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="text-sm font-semibold text-slate-100">
                {ready ? t("preflight.ready") : t("preflight.almost", { n: String(issues) })}
              </h2>
              {route ? (
                <p className="mt-0.5 flex items-center gap-1.5 text-xs text-slate-400">
                  <Plane className="h-3 w-3" />
                  <span className="font-mono font-semibold text-slate-300">
                    {route.originIcao}
                  </span>
                  <span className="text-slate-600">→</span>
                  <span className="font-mono font-semibold text-slate-300">
                    {route.destinationIcao}
                  </span>
                  {route.aircraftIcao && (
                    <span className="ml-1 truncate text-slate-500">
                      · {route.aircraftIcao}
                    </span>
                  )}
                  <span className="ml-1 rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-slate-500">
                    {route.fromPlan ? t("preflight.from_ofp") : t("preflight.from_sim")}
                  </span>
                </p>
              ) : (
                <p className="mt-0.5 text-xs text-slate-500">
                  {t("preflight.no_route")}
                </p>
              )}
            </div>
            <button
              onClick={onClose}
              className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="flex-1 overflow-y-auto px-5 py-4">
            {/* Checklist */}
            <ul className="space-y-2">
              {checks.map((c) => (
                <li
                  key={c.id}
                  className="flex items-start gap-3 rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-2.5"
                >
                  <span className="mt-0.5 shrink-0">
                    {c.status === "ok" && (
                      <CheckCircle2 className="h-4 w-4 text-emerald-400" />
                    )}
                    {c.status === "warn" && (
                      <AlertTriangle className="h-4 w-4 text-amber-400" />
                    )}
                    {c.status === "info" && (
                      <Info className="h-4 w-4 text-sky-400" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-medium text-slate-200">
                      {t(c.titleKey, c.titleArgs)}
                    </p>
                    {c.detailKey && (
                      <p className="mt-0.5 text-[11px] leading-snug text-slate-500">
                        {t(c.detailKey, c.detailArgs)}
                      </p>
                    )}
                    {/* (v6.2.34) Bypass "usar escenario por defecto" — para
                        cuando el escenario no existe o no lo quieres. */}
                    {c.bypassIcao && (
                      <label className="mt-1.5 inline-flex cursor-pointer items-center gap-1.5 text-[11px] text-slate-400 hover:text-slate-200">
                        <input
                          type="checkbox"
                          className="h-3 w-3 accent-emerald-500"
                          checked={c.status === "ok"}
                          onChange={(e) =>
                            setBypass(c.bypassIcao as string, e.target.checked)
                          }
                        />
                        {t("preflight.use_default")}
                      </label>
                    )}
                  </div>
                  {c.action && c.actionKey && (
                    <button
                      onClick={() => void runAction(c)}
                      disabled={busyId === c.id}
                      className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-slate-800 px-2.5 py-1.5 text-[11px] font-semibold text-slate-100 hover:bg-slate-700 disabled:opacity-50"
                    >
                      {busyId === c.id ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : c.action.kind === "search" ? (
                        <Search className="h-3 w-3" />
                      ) : (
                        <RefreshCw className="h-3 w-3" />
                      )}
                      {t(c.actionKey)}
                    </button>
                  )}
                </li>
              ))}
            </ul>

            {/* Radar de escenarios nuevos */}
            <div className="mt-5">
              <div className="mb-2 flex items-center justify-between gap-2">
                <h3 className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                  <Sparkles className="h-3.5 w-3.5 text-fuchsia-300" />
                  {t("preflight.radar.title")}
                  {fresh.length > 0 && (
                    <span className="rounded-full bg-fuchsia-500/20 px-1.5 py-0.5 text-[10px] font-bold text-fuchsia-200">
                      {fresh.length}
                    </span>
                  )}
                </h3>
                {fresh.length > 0 && (
                  <button
                    onClick={() => acknowledgeAll()}
                    className="text-[11px] font-medium text-slate-400 hover:text-slate-200"
                  >
                    {t("preflight.radar.mark_seen")}
                  </button>
                )}
              </div>

              {radarLoading && fresh.length === 0 && (
                <div className="inline-flex items-center gap-2 text-xs text-slate-500">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("preflight.radar.loading")}
                </div>
              )}

              {!radarLoading && fresh.length === 0 && (
                <p className="rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-3 text-[11px] text-slate-500">
                  {t("preflight.radar.empty")}
                </p>
              )}

              {fresh.length > 0 && (
                <ul className="space-y-2">
                  {fresh.slice(0, 8).map((a) => (
                    <li
                      key={a.id}
                      className="group flex items-center gap-3 overflow-hidden rounded-xl border border-slate-800 bg-slate-900/40 pr-2 transition-colors hover:border-fuchsia-500/30"
                    >
                      <button
                        onClick={() => openScenery(a.icao, a.title)}
                        className="flex min-w-0 flex-1 items-center gap-3 py-2 pl-2.5 text-left"
                      >
                        {a.imageUrl ? (
                          <img
                            src={a.imageUrl}
                            alt=""
                            loading="lazy"
                            className="h-10 w-14 shrink-0 rounded-md object-cover ring-1 ring-slate-800"
                          />
                        ) : (
                          <div className="flex h-10 w-14 shrink-0 items-center justify-center rounded-md bg-slate-800 text-slate-600">
                            <Plane className="h-4 w-4" />
                          </div>
                        )}
                        <div className="min-w-0 flex-1">
                          <p className="truncate text-xs font-medium text-slate-200">
                            {a.title}
                          </p>
                          <p className="mt-0.5 flex items-center gap-1.5 text-[10px] text-slate-500">
                            {a.icao && (
                              <span className="rounded bg-slate-800 px-1 py-0.5 font-mono text-slate-400">
                                {a.icao}
                              </span>
                            )}
                            {a.developer && <span className="truncate">{a.developer}</span>}
                            {a.releasedAt && (
                              <span className="ml-auto shrink-0 text-slate-600">
                                {fmtDate(a.releasedAt)}
                              </span>
                            )}
                          </p>
                        </div>
                      </button>
                      <button
                        onClick={() => api.openExternal(a.pageUrl)}
                        title={t("preflight.radar.open")}
                        className="rounded-md p-1.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
                      >
                        <ExternalLink className="h-3.5 w-3.5" />
                      </button>
                      <button
                        onClick={() => dismissScenery(a.id)}
                        title={t("preflight.radar.dismiss")}
                        className="rounded-md p-1.5 text-slate-600 hover:bg-slate-800 hover:text-slate-300"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
