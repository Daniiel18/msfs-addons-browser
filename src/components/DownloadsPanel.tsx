import { useMemo } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Loader2,
  Pause,
  Play,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import type {
  CommunityInfo,
  DownloadJob,
  DownloadPhase,
  InstalledAddon,
} from "../lib/types";
import { toJobsList, useDownloadsStore } from "../stores/useDownloadsStore";
import { api } from "../lib/tauri";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function DownloadsPanel({ open, onClose }: Props) {
  const jobsMap = useDownloadsStore((s) => s.jobs);
  const jobs = useMemo(() => toJobsList(jobsMap), [jobsMap]);
  const installed = useDownloadsStore((s) => s.installed);
  const community = useDownloadsStore((s) => s.community);
  const tab = useDownloadsStore((s) => s.panelTab);
  const setTab = useDownloadsStore((s) => s.setPanelTab);

  // AnimatePresence only tracks *direct* children — wrapping the scrim +
  // aside in a Fragment silently breaks mount/unmount tracking, which can
  // leave the scrim stuck over the UI. Keep them as sibling presence blocks.
  return (
    <>
      <AnimatePresence>
        {open && (
          <motion.div
            key="scrim"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={onClose}
            className="fixed inset-0 z-30 bg-slate-950/70"
          />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {open && (
          <motion.aside
            key="panel"
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%" }}
            transition={{ type: "spring", stiffness: 320, damping: 34 }}
            className="fixed right-0 top-0 z-40 flex h-screen w-full max-w-md flex-col border-l border-slate-800 bg-slate-950 shadow-2xl"
          >
            <header className="flex items-center justify-between border-b border-slate-800 px-5 py-4">
              <div className="flex items-center gap-2">
                <Download className="h-4 w-4 text-brand-300" />
                <h2 className="text-sm font-semibold text-slate-100">Descargas</h2>
              </div>
              <button
                onClick={onClose}
                className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
                title="Cerrar"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            <CommunityBanner info={community} />

            {/* Tabs: «Activas» (jobs en curso o recientes) vs
                «Instalados» (historial persistente). Mantenemos un
                único panel para que el usuario no pierda contexto al
                pasar de uno a otro. */}
            <div className="flex border-b border-slate-800 px-5">
              <TabButton
                active={tab === "active"}
                onClick={() => setTab("active")}
                badge={jobs.length || undefined}
              >
                Activas
              </TabButton>
              <TabButton
                active={tab === "installed"}
                onClick={() => setTab("installed")}
                badge={installed.length || undefined}
              >
                Instalados
              </TabButton>
            </div>

            <div className="flex-1 overflow-y-auto px-4 py-3">
              {tab === "active" ? (
                jobs.length === 0 ? (
                  <ActiveEmptyState />
                ) : (
                  <ul className="space-y-2">
                    {jobs.map((j) => (
                      <JobRow key={j.id} job={j} />
                    ))}
                  </ul>
                )
              ) : installed.length === 0 ? (
                <InstalledEmptyState />
              ) : (
                <ul className="space-y-2">
                  {installed.map((it) => (
                    <InstalledRow key={it.id} item={it} />
                  ))}
                </ul>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>
    </>
  );
}

function TabButton({
  active,
  onClick,
  badge,
  children,
}: {
  active: boolean;
  onClick: () => void;
  badge?: number;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`relative -mb-px flex items-center gap-2 border-b-2 px-3 py-2.5 text-xs font-medium transition-colors ${
        active
          ? "border-brand-400 text-slate-100"
          : "border-transparent text-slate-400 hover:text-slate-200"
      }`}
    >
      {children}
      {badge !== undefined && badge > 0 && (
        <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] text-slate-400">
          {badge}
        </span>
      )}
    </button>
  );
}

function ActiveEmptyState() {
  return (
    <div className="mt-16 flex flex-col items-center text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-slate-900 ring-1 ring-slate-800">
        <Download className="h-5 w-5 text-slate-500" />
      </div>
      <p className="mt-3 text-sm text-slate-400">Aún no hay descargas</p>
      <p className="mt-1 max-w-[260px] text-xs text-slate-500">
        Toca un botón de descarga en cualquier resultado para empezar. El progreso aparecerá aquí.
      </p>
    </div>
  );
}

function InstalledEmptyState() {
  return (
    <div className="mt-16 flex flex-col items-center text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-slate-900 ring-1 ring-slate-800">
        <Folder className="h-5 w-5 text-slate-500" />
      </div>
      <p className="mt-3 text-sm text-slate-400">Sin paquetes instalados</p>
      <p className="mt-1 max-w-[260px] text-xs text-slate-500">
        Cuando instales un addon (por torrent o desde un archivo local) aparecerá aquí.
      </p>
    </div>
  );
}

function CommunityBanner({ info }: { info: CommunityInfo | null }) {
  if (!info) {
    return (
      <div className="flex items-start gap-2 border-b border-amber-500/20 bg-amber-500/10 px-5 py-2.5 text-xs text-amber-200">
        <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <div>
          No se detectó la carpeta Community de MSFS 2020 automáticamente. Configúrala desde
          Ajustes (próximamente) para habilitar la auto-instalación.
        </div>
      </div>
    );
  }
  return (
    <div className="flex items-start gap-2 border-b border-slate-800 bg-slate-900/60 px-5 py-2.5 text-[11px] text-slate-400">
      <Folder className="mt-0.5 h-3.5 w-3.5 shrink-0 text-brand-300" />
      <div className="min-w-0">
        <div className="text-slate-300">
          Community ({info.variant === "msstore" ? "MS Store" : "Steam"})
          {!info.exists && (
            <span className="ml-1 text-amber-300">· no existe aún</span>
          )}
        </div>
        <div className="truncate font-mono text-[10px] text-slate-500">{info.path}</div>
      </div>
    </div>
  );
}

function JobRow({ job }: { job: DownloadJob }) {
  const cancel = useDownloadsStore((s) => s.cancel);
  const pause = useDownloadsStore((s) => s.pause);
  const resume = useDownloadsStore((s) => s.resume);
  const clear = useDownloadsStore((s) => s.clear);

  const terminal =
    job.phase === "completed" ||
    job.phase === "cancelled" ||
    job.phase === "error";

  // Pause/resume only makes sense for torrent jobs that are actively
  // downloading (or already paused). Mirror/direct jobs get nothing —
  // the browser owns the download there, we'd just confuse the user.
  const canPause = job.methodKind === "torrent" && job.phase === "downloading";
  const canResume = job.methodKind === "torrent" && job.phase === "paused";

  return (
    <motion.li
      layout
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="rounded-xl border border-slate-800 bg-slate-900/60 p-3"
    >
      <div className="flex items-start gap-3">
        <PhaseIcon phase={job.phase} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-slate-100">
            {job.addonTitle}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-[11px] text-slate-500">
            <span className="uppercase tracking-wide">{job.source}</span>
            <span>·</span>
            <span>{job.methodName}</span>
          </div>
          {(job.phase === "downloading" ||
            job.phase === "paused" ||
            job.phase === "installing") &&
            job.bytesTotal > 0 && (
              <>
                <ProgressBar done={job.bytesDone} total={job.bytesTotal} />
                <ProgressDetails job={job} />
              </>
            )}
          {job.message && (
            <p className="mt-1.5 text-xs text-slate-400">{job.message}</p>
          )}
          {job.error && (
            <p className="mt-1.5 text-xs text-red-300/90">{job.error}</p>
          )}
          {job.phase === "completed" && job.installPath && (
            <button
              onClick={() => api.openLocalPath(job.installPath!)}
              className="mt-2 inline-flex items-center gap-1 text-[11px] text-brand-300 hover:text-brand-200"
            >
              <FolderOpen className="h-3 w-3" />
              Abrir carpeta instalada
            </button>
          )}
          {job.phase === "completed" && job.methodKind !== "torrent" && (
            <button
              onClick={() => api.openExternal(job.url)}
              className="mt-2 inline-flex items-center gap-1 text-[11px] text-brand-300 hover:text-brand-200"
            >
              <ExternalLink className="h-3 w-3" />
              Reabrir enlace
            </button>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {canPause && (
            <button
              onClick={() => pause(job.id)}
              className="rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-amber-300"
              title="Pausar"
            >
              <Pause className="h-3.5 w-3.5" />
            </button>
          )}
          {canResume && (
            <button
              onClick={() => resume(job.id)}
              className="rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-brand-300"
              title="Reanudar"
            >
              <Play className="h-3.5 w-3.5" />
            </button>
          )}
          {!terminal && (
            <button
              onClick={() => cancel(job.id)}
              className="rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
              title="Cancelar"
            >
              <XCircle className="h-3.5 w-3.5" />
            </button>
          )}
          {terminal && (
            <button
              onClick={() => clear(job.id)}
              className="rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-red-300"
              title="Quitar de la lista"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>
    </motion.li>
  );
}

/**
 * Una fila del historial persistente.
 *
 * A diferencia de `JobRow`, aquí no hay máquina de estados: la
 * instalación ya ocurrió. Las acciones disponibles son leer-only
 * («Abrir carpeta») u olvidar la fila del historial sin tocar el
 * disco.
 */
function InstalledRow({ item }: { item: InstalledAddon }) {
  const forget = useDownloadsStore((s) => s.forgetInstalled);

  const sizeLabel = item.sizeBytes !== null ? formatBytes(item.sizeBytes) : null;
  const sourceLabel = item.source ? item.source : "manual";

  return (
    <motion.li
      layout
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="rounded-xl border border-slate-800 bg-slate-900/60 p-3"
    >
      <div className="flex items-start gap-3">
        <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-brand-300" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-slate-100">
            {item.title}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-slate-500">
            <span className="uppercase tracking-wide">{sourceLabel}</span>
            {item.developer && (
              <>
                <span>·</span>
                <span>{item.developer}</span>
              </>
            )}
            {item.version && (
              <>
                <span>·</span>
                <span>v{item.version}</span>
              </>
            )}
            {sizeLabel && (
              <>
                <span>·</span>
                <span>{sizeLabel}</span>
              </>
            )}
            <span>·</span>
            <span title={item.installedAt}>{formatRelativeDate(item.installedAt)}</span>
          </div>
          <div className="mt-1 truncate font-mono text-[10px] text-slate-500">
            {item.installPath}
          </div>
          <div className="mt-2 flex items-center gap-3">
            <button
              onClick={() => api.openLocalPath(item.installPath)}
              className="inline-flex items-center gap-1 text-[11px] text-brand-300 hover:text-brand-200"
            >
              <FolderOpen className="h-3 w-3" />
              Abrir carpeta
            </button>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => forget(item.id)}
            className="rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-red-300"
            title="Quitar del historial (no borra el disco)"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </motion.li>
  );
}

function PhaseIcon({ phase }: { phase: DownloadPhase }) {
  const common = "mt-0.5 h-4 w-4 shrink-0";
  switch (phase) {
    case "queued":
    case "resolving":
    case "downloading":
    case "installing":
      return <Loader2 className={`${common} animate-spin text-brand-300`} />;
    case "paused":
      // Static (non-spinning) icon communicates "nothing is happening
      // right now" more clearly than a frozen-looking Loader2.
      return <Pause className={`${common} text-amber-300`} />;
    case "completed":
      return <CheckCircle2 className={`${common} text-brand-300`} />;
    case "cancelled":
      return <XCircle className={`${common} text-slate-500`} />;
    case "error":
      return <AlertCircle className={`${common} text-red-400`} />;
  }
}

function ProgressBar({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.min(100, (done / total) * 100) : 0;
  return (
    <div className="mt-2 h-1 overflow-hidden rounded-full bg-slate-800">
      <div
        className="h-full bg-brand-500 transition-[width] duration-200"
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

/**
 * Compact "123.4 MB / 789.0 MB · 2.3 MB/s · 45s" line under the progress
 * bar. Kept tiny so multiple concurrent jobs can stack cleanly in the
 * side panel.
 */
function ProgressDetails({ job }: { job: DownloadJob }) {
  const pct = job.bytesTotal > 0 ? (job.bytesDone / job.bytesTotal) * 100 : 0;
  const parts: string[] = [];
  parts.push(`${formatBytes(job.bytesDone)} / ${formatBytes(job.bytesTotal)}`);
  if (job.phase === "downloading") {
    if (job.speedBps > 0) parts.push(`${formatBytes(job.speedBps)}/s`);
    if (job.etaSeconds !== null && job.etaSeconds > 0) parts.push(`${formatDuration(job.etaSeconds)} restante`);
  }
  return (
    <div className="mt-1 flex items-center justify-between text-[10px] text-slate-500">
      <span>{parts.join(" · ")}</span>
      <span className="tabular-nums">{pct.toFixed(1)}%</span>
    </div>
  );
}

/** Human-friendly byte formatting (binary multiples — matches OS file sizes). */
function formatBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))}s`;
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60);
    const s = Math.round(seconds % 60);
    return `${m}m ${s}s`;
  }
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return `${h}h ${m}m`;
}

/**
 * Convierte el TEXT que llega de SQLite (`datetime('now')` produce
 * "YYYY-MM-DD HH:MM:SS" en UTC) en una etiqueta relativa amigable.
 *
 * Dejamos los saltos en español: «hace 5m», «hace 3h», «hace 2d».
 */
function formatRelativeDate(iso: string): string {
  // SQLite emite "YYYY-MM-DD HH:MM:SS" sin la 'T' ni el sufijo 'Z'.
  // El constructor de Date lo trata como local en algunos navegadores,
  // así que lo normalizamos a ISO-8601 UTC antes de parsear.
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const t = Date.parse(normalized);
  if (Number.isNaN(t)) return iso;
  const diffMs = Date.now() - t;
  if (diffMs < 0) return "ahora";
  const sec = Math.floor(diffMs / 1000);
  if (sec < 60) return "hace unos segundos";
  const min = Math.floor(sec / 60);
  if (min < 60) return `hace ${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `hace ${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `hace ${day}d`;
  const month = Math.floor(day / 30);
  if (month < 12) return `hace ${month} mes${month === 1 ? "" : "es"}`;
  const year = Math.floor(day / 365);
  return `hace ${year} año${year === 1 ? "" : "s"}`;
}
