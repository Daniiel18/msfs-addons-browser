import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertTriangle,
  FolderOpen,
  HardDrive,
  Loader2,
  Plane,
  Star,
  Trash2,
  Video,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import { t, getActiveLocale } from "../lib/i18n";
import { isAirport } from "../lib/packageType";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useToastStore } from "../stores/useToastStore";
import type { LandingClip } from "../lib/types";

interface Props {
  onClose: () => void;
}

function formatBytes(n: number): string {
  if (!isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/**
 * (v6.2.29 / R6) Limpieza inteligente de disco.
 *
 * Cruza los escenarios de aeropuerto INSTALADOS con tu historial de
 * vuelos: los que nunca has volado (su ICAO no aparece como origen ni
 * destino en el FlightBook) y ocupan más disco son candidatos a
 * desinstalar. Segunda sección: grabaciones de aterrizaje guardadas,
 * para borrar las que ya no quieras (respetando las favoritas).
 *
 * Acciones destructivas (desinstalar / borrar clip) piden confirmación
 * inline. Desinstalar usa `uninstall_community_package` (borra de disco
 * + limpia DB) y luego re-escanea Community.
 */
export function DiskCleanupModal({ onClose }: Props) {
  const packages = useCommunityStore((s) => s.packages);
  const rescan = useCommunityStore((s) => s.rescan);
  const entries = useFlightLogStore((s) => s.entries);
  const pushToast = useToastStore((s) => s.push);

  const [clips, setClips] = useState<LandingClip[] | null>(null);
  const [confirmPkg, setConfirmPkg] = useState<string | null>(null);
  const [busyPkg, setBusyPkg] = useState<string | null>(null);
  const [confirmClip, setConfirmClip] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    let cancelled = false;
    api
      .listLandingClips()
      .then((c) => {
        if (!cancelled) setClips(c);
      })
      .catch(() => {
        if (!cancelled) setClips([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // ICAOs visitados (origen o destino) → cuántas veces.
  const visited = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entries) {
      for (const icao of [e.originIcao, e.destinationIcao]) {
        if (icao) {
          const k = icao.toUpperCase();
          m.set(k, (m.get(k) ?? 0) + 1);
        }
      }
    }
    return m;
  }, [entries]);

  // Escenarios de aeropuerto nunca volados, mayor tamaño primero.
  const neverFlown = useMemo(() => {
    return packages
      .filter((p) => isAirport(p) && (visited.get((p.icao ?? "").toUpperCase()) ?? 0) === 0)
      .sort((a, b) => (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0));
  }, [packages, visited]);

  const reclaimable = useMemo(
    () => neverFlown.reduce((sum, p) => sum + (p.sizeBytes ?? 0), 0),
    [neverFlown],
  );

  const sortedClips = useMemo(() => {
    if (!clips) return [];
    // Candidatos primero: no-favoritos y más antiguos arriba.
    return [...clips].sort((a, b) => {
      if (a.favorite !== b.favorite) return a.favorite ? 1 : -1;
      return (a.recordedAt ?? "").localeCompare(b.recordedAt ?? "");
    });
  }, [clips]);

  const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
  const fmtDate = (iso: string) => {
    const d = new Date(iso);
    return isNaN(d.getTime())
      ? ""
      : d.toLocaleDateString(locale, { day: "2-digit", month: "short", year: "numeric" });
  };
  const fmtDur = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = Math.round(s % 60);
    return `${m}:${String(sec).padStart(2, "0")}`;
  };

  const uninstall = async (folderName: string) => {
    setBusyPkg(folderName);
    try {
      const report = await api.uninstallCommunityPackage(folderName);
      if (report.failedPaths.length > 0) {
        pushToast({
          kind: "error",
          title: t("cleanup.uninstall_err"),
          message: report.failedPaths[0].error,
        });
      } else {
        pushToast({ kind: "success", title: t("cleanup.uninstalled"), ttlMs: 2200 });
      }
      await rescan();
    } catch (e) {
      pushToast({ kind: "error", title: t("cleanup.uninstall_err"), message: String(e) });
    } finally {
      setBusyPkg(null);
      setConfirmPkg(null);
    }
  };

  const deleteClip = async (id: string) => {
    try {
      await api.deleteLandingClip(id);
      setClips((prev) => (prev ? prev.filter((c) => c.id !== id) : prev));
      pushToast({ kind: "success", title: t("cleanup.clips.deleted"), ttlMs: 2000 });
    } catch (e) {
      pushToast({ kind: "error", title: t("cleanup.clips.delete_err"), message: String(e) });
    } finally {
      setConfirmClip(null);
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        key="cleanup-backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
        className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      >
        <motion.div
          key="cleanup-modal"
          initial={{ opacity: 0, scale: 0.96, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          className="relative flex max-h-[calc(100vh-3rem)] w-[min(680px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <header className="flex items-center gap-3 border-b border-slate-800 px-5 py-3.5">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sky-500/20 text-sky-300">
              <HardDrive className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="text-sm font-semibold text-slate-100">
                {t("cleanup.title")}
              </h2>
              <p className="mt-0.5 text-xs text-slate-400">
                {reclaimable > 0
                  ? t("cleanup.subtitle", { size: formatBytes(reclaimable) })
                  : t("cleanup.subtitle_none")}
              </p>
            </div>
            <button
              onClick={onClose}
              className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="flex-1 overflow-y-auto px-5 py-4">
            {/* Escenarios nunca volados */}
            <h3 className="mb-2 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              <Plane className="h-3.5 w-3.5 text-amber-300" />
              {t("cleanup.scenery.title")}
              {neverFlown.length > 0 && (
                <span className="rounded-full bg-amber-500/20 px-1.5 py-0.5 text-[10px] font-bold text-amber-200">
                  {neverFlown.length}
                </span>
              )}
            </h3>

            {neverFlown.length === 0 ? (
              <p className="rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-3 text-[11px] text-slate-500">
                {t("cleanup.scenery.empty")}
              </p>
            ) : (
              <ul className="space-y-2">
                {neverFlown.map((p) => (
                  <li
                    key={p.folderName}
                    className="flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-2.5"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="flex items-center gap-1.5 truncate text-xs font-medium text-slate-200">
                        {p.icao && (
                          <span className="rounded bg-slate-800 px-1 py-0.5 font-mono text-[10px] text-slate-400">
                            {p.icao}
                          </span>
                        )}
                        <span className="truncate">{p.title}</span>
                      </p>
                      <p className="mt-0.5 text-[11px] text-slate-500">
                        {formatBytes(p.sizeBytes ?? 0)} · {t("cleanup.scenery.never")}
                      </p>
                    </div>
                    <button
                      onClick={() => void api.openLocalPath(p.installPath)}
                      title={t("cleanup.reveal")}
                      className="shrink-0 rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
                    >
                      <FolderOpen className="h-3.5 w-3.5" />
                    </button>
                    {confirmPkg === p.folderName ? (
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          onClick={() => void uninstall(p.folderName)}
                          disabled={busyPkg === p.folderName}
                          className="inline-flex items-center gap-1 rounded-md bg-rose-500 px-2 py-1.5 text-[11px] font-bold text-white hover:bg-rose-400 disabled:opacity-50"
                        >
                          {busyPkg === p.folderName ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <AlertTriangle className="h-3 w-3" />
                          )}
                          {t("cleanup.confirm_yes")}
                        </button>
                        <button
                          onClick={() => setConfirmPkg(null)}
                          className="rounded-md px-2 py-1.5 text-[11px] text-slate-400 hover:text-slate-200"
                        >
                          {t("cleanup.cancel")}
                        </button>
                      </div>
                    ) : (
                      <button
                        onClick={() => setConfirmPkg(p.folderName)}
                        className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-slate-800 px-2.5 py-1.5 text-[11px] font-semibold text-rose-200 hover:bg-slate-700"
                      >
                        <Trash2 className="h-3 w-3" />
                        {t("cleanup.uninstall")}
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}

            {/* Grabaciones de aterrizaje */}
            <h3 className="mb-2 mt-5 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              <Video className="h-3.5 w-3.5 text-fuchsia-300" />
              {t("cleanup.clips.title")}
              {sortedClips.length > 0 && (
                <span className="rounded-full bg-fuchsia-500/20 px-1.5 py-0.5 text-[10px] font-bold text-fuchsia-200">
                  {sortedClips.length}
                </span>
              )}
            </h3>

            {clips === null ? (
              <div className="inline-flex items-center gap-2 text-xs text-slate-500">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("cleanup.clips.loading")}
              </div>
            ) : sortedClips.length === 0 ? (
              <p className="rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-3 text-[11px] text-slate-500">
                {t("cleanup.clips.empty")}
              </p>
            ) : (
              <>
                <p className="mb-2 text-[10px] text-slate-600">{t("cleanup.clips.hint")}</p>
                <ul className="space-y-2">
                  {sortedClips.map((c) => (
                    <li
                      key={c.id}
                      className="flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/40 px-3.5 py-2.5"
                    >
                      <div className="min-w-0 flex-1">
                        <p className="flex items-center gap-1.5 truncate text-xs font-medium text-slate-200">
                          {c.favorite && <Star className="h-3 w-3 shrink-0 text-amber-400" fill="currentColor" />}
                          <span className="truncate">
                            {c.airportIcao ? `${c.airportIcao} · ` : ""}
                            {c.model ?? t("cleanup.clips.clip")}
                          </span>
                          {c.isTest && (
                            <span className="rounded bg-slate-800 px-1 py-0.5 text-[9px] uppercase text-slate-500">
                              {t("cleanup.clips.test")}
                            </span>
                          )}
                        </p>
                        <p className="mt-0.5 text-[11px] text-slate-500">
                          {fmtDate(c.recordedAt)} · {fmtDur(c.durationS)}
                        </p>
                      </div>
                      <button
                        onClick={() => void api.openLocalPath(c.path)}
                        title={t("cleanup.reveal")}
                        className="shrink-0 rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
                      >
                        <FolderOpen className="h-3.5 w-3.5" />
                      </button>
                      {confirmClip === c.id ? (
                        <div className="flex shrink-0 items-center gap-1">
                          <button
                            onClick={() => void deleteClip(c.id)}
                            className="rounded-md bg-rose-500 px-2 py-1.5 text-[11px] font-bold text-white hover:bg-rose-400"
                          >
                            {t("cleanup.confirm_yes")}
                          </button>
                          <button
                            onClick={() => setConfirmClip(null)}
                            className="rounded-md px-2 py-1.5 text-[11px] text-slate-400 hover:text-slate-200"
                          >
                            {t("cleanup.cancel")}
                          </button>
                        </div>
                      ) : (
                        <button
                          onClick={() => setConfirmClip(c.id)}
                          title={t("cleanup.clips.delete")}
                          className="shrink-0 rounded-md p-1.5 text-slate-500 hover:bg-slate-800 hover:text-rose-300"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
