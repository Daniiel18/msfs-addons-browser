import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Link2,
  Loader2,
  Package,
  Pause,
  Play,
  RotateCcw,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import type {
  Addon,
  CommunityInfo,
  DownloadJob,
  DownloadMethod,
  InstalledAddon,
} from "../lib/types";
import { toJobsList, useDownloadsStore } from "../stores/useDownloadsStore";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

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
            // (Rediseño F5) Bajo la barra superior (top-11): los controles de
            // ventana y los accesos globales siguen a mano con el panel abierto.
            className="fixed inset-x-0 bottom-0 top-11 z-[46] bg-slate-950/70"
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
            data-sf-overlay
            className="fixed bottom-0 right-0 top-11 z-[47] flex w-full max-w-md flex-col border-l border-sf-border bg-sf-bg shadow-2xl"
          >
            <header className="flex items-center justify-between border-b border-sf-border px-5 py-4">
              <div className="flex items-center gap-2">
                <Download className="h-4 w-4 text-brand-300" />
                <h2 className="text-sm font-semibold text-sf-text">{t("nav.downloads")}</h2>
              </div>
              <button
                onClick={onClose}
                className="rounded-lg p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                title={t("common.close")}
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            <CommunityBanner info={community} />

            {/* Tabs: «Activas» (jobs en curso o recientes) vs
                «Instalados» (historial persistente). Mantenemos un
                único panel para que el usuario no pierda contexto al
                pasar de uno a otro. */}
            <div className="flex border-b border-sf-border px-5">
              <TabButton
                active={tab === "active"}
                onClick={() => setTab("active")}
                badge={jobs.length || undefined}
              >
                {t("downloads.tab.active")}
              </TabButton>
              <TabButton
                active={tab === "installed"}
                onClick={() => setTab("installed")}
                badge={installed.length || undefined}
              >
                {t("downloads.tab.installed")}
              </TabButton>
            </div>

            <div className="flex-1 overflow-y-auto px-4 py-3">
              {tab === "active" ? (
                jobs.length === 0 ? (
                  <ActiveEmptyState />
                ) : (
                  <JobSections jobs={jobs} />
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
          ? "border-brand-400 text-sf-text"
          : "border-transparent text-sf-muted hover:text-sf-text"
      }`}
    >
      {children}
      {badge !== undefined && badge > 0 && (
        <span className="rounded bg-sf-raised px-1.5 py-0.5 text-[11px] text-sf-muted">
          {badge}
        </span>
      )}
    </button>
  );
}

function ActiveEmptyState() {
  return (
    <div className="mt-16 flex flex-col items-center text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-sf-surface ring-1 ring-sf-border">
        <Download className="h-5 w-5 text-sf-muted" />
      </div>
      <p className="mt-3 text-sm text-sf-muted">{t("downloads.empty.title")}</p>
      <p className="mt-1 max-w-[260px] text-xs text-sf-muted">
        {t("downloads.empty.hint")}
      </p>
    </div>
  );
}

function InstalledEmptyState() {
  return (
    <div className="mt-16 flex flex-col items-center text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-sf-surface ring-1 ring-sf-border">
        <Folder className="h-5 w-5 text-sf-muted" />
      </div>
      <p className="mt-3 text-sm text-sf-muted">{t("downloads.installed.empty.title")}</p>
      <p className="mt-1 max-w-[260px] text-xs text-sf-muted">
        {t("downloads.installed.empty.hint")}
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
          {t("downloads.community.not_found")}
        </div>
      </div>
    );
  }
  return (
    <div className="flex items-start gap-2 border-b border-sf-border bg-sf-surface px-5 py-2.5 text-[11px] text-sf-muted">
      <Folder className="mt-0.5 h-3.5 w-3.5 shrink-0 text-brand-300" />
      <div className="min-w-0">
        <div className="text-sf-text/90">
          Community ({info.variant === "msstore" ? "MS Store" : "Steam"})
          {!info.exists && (
            <span className="ml-1 text-amber-300">· {t("downloads.community.not_exists")}</span>
          )}
        </div>
        <div className="truncate font-mono text-[11px] text-sf-muted">{info.path}</div>
      </div>
    </div>
  );
}

const isTerminal = (j: DownloadJob) =>
  j.phase === "completed" || j.phase === "cancelled" || j.phase === "error";

/** (25/10) «En curso» arriba y el «Historial» (7 días) plegable debajo. */
function JobSections({ jobs }: { jobs: DownloadJob[] }) {
  const clear = useDownloadsStore((s) => s.clear);
  const [historyOpen, setHistoryOpen] = useState(true);
  const active = jobs.filter((j) => !isTerminal(j));
  const history = jobs.filter(isTerminal);
  return (
    <div className="space-y-4">
      {active.length > 0 && (
        <section>
          <SectionTitle>
            {t("downloads.section.active")} <span className="text-sf-muted">({active.length})</span>
          </SectionTitle>
          <ul className="space-y-2">
            <AnimatePresence initial={false}>
              {active.map((j) => (
                <JobRow key={j.id} job={j} />
              ))}
            </AnimatePresence>
          </ul>
        </section>
      )}
      {history.length > 0 && (
        <section>
          <div className="mb-2 flex items-center justify-between">
            <button
              onClick={() => setHistoryOpen((v) => !v)}
              className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-sf-muted hover:text-sf-text"
            >
              <ChevronDown className={`h-3.5 w-3.5 transition-transform ${historyOpen ? "" : "-rotate-90"}`} />
              {t("downloads.section.history")} <span>({history.length})</span>
            </button>
            <button
              onClick={() => history.forEach((j) => void clear(j.id))}
              className="rounded-md px-2 py-0.5 text-[11px] text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            >
              {t("downloads.clear_history")}
            </button>
          </div>
          {historyOpen && (
            <ul className="space-y-2">
              <AnimatePresence initial={false}>
                {history.map((j) => (
                  <JobRow key={j.id} job={j} />
                ))}
              </AnimatePresence>
            </ul>
          )}
        </section>
      )}
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-sf-text/80">{children}</h3>
  );
}

/** (25/10) Miniatura del addon (la de su catálogo) o un icono. */
function JobThumb({ job }: { job: DownloadJob }) {
  const [failed, setFailed] = useState(false);
  if (job.imageUrl && !failed) {
    return (
      <img
        src={job.imageUrl}
        alt=""
        loading="lazy"
        referrerPolicy="no-referrer"
        onError={() => setFailed(true)}
        className="h-12 w-16 shrink-0 rounded-lg object-cover ring-1 ring-sf-border"
      />
    );
  }
  return (
    <div className="flex h-12 w-16 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-slate-800 to-slate-950 ring-1 ring-sf-border">
      <Package className="h-5 w-5 text-slate-500" strokeWidth={1.5} />
    </div>
  );
}

function JobRow({ job }: { job: DownloadJob }) {
  const cancel = useDownloadsStore((s) => s.cancel);
  const pause = useDownloadsStore((s) => s.pause);
  const resume = useDownloadsStore((s) => s.resume);
  const clear = useDownloadsStore((s) => s.clear);
  const retry = useDownloadsStore((s) => s.retry);
  const [busy, setBusy] = useState(false);

  const terminal = isTerminal(job);
  const failed = job.phase === "error" || job.phase === "cancelled";
  const canPause = job.pausable && job.phase === "downloading";
  const canResume = job.phase === "paused";
  const canRetry = failed && (job.resumable || !!job.addonId);
  const showBar = job.phase === "downloading" || job.phase === "paused" || job.phase === "installing";
  const run = (f: () => Promise<void>) => {
    setBusy(true);
    void f().finally(() => setBusy(false));
  };

  return (
    <motion.li
      layout
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className={`rounded-xl border bg-sf-surface p-3 transition-colors ${
        job.phase === "error"
          ? "border-rose-500/30"
          : job.phase === "paused"
            ? "border-amber-500/30"
            : "border-sf-border hover:border-sf-border/80"
      }`}
    >
      <div className="flex items-start gap-3">
        <div className="relative">
          <JobThumb job={job} />
          <span className="absolute -bottom-1 -right-1 rounded-full bg-sf-bg p-0.5">
            <PhaseIcon phase={job.phase} />
          </span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-sf-text" title={job.addonTitle}>
            {job.addonTitle}
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-sf-muted">
            <span className="shrink-0 rounded bg-sf-raised px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide text-sf-text/80">
              {job.source}
            </span>
            <span className="truncate">{job.methodName}</span>
          </div>
          {showBar && <ProgressBar job={job} />}
          {showBar && <ProgressDetails job={job} />}
          {job.message && !showBar && (
            <p className="mt-1.5 text-xs text-sf-muted">{translateJobMessage(job.message)}</p>
          )}
          {job.error && <p className="mt-1.5 text-xs leading-snug text-rose-300/90">{job.error}</p>}
          {(canRetry || !!job.addonId && failed || job.phase === "completed") && (
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {canRetry && (
                <button
                  disabled={busy}
                  onClick={() => run(() => retry(job))}
                  className="inline-flex items-center gap-1 rounded-md bg-brand-500/15 px-2 py-1 text-[11px] font-semibold text-brand-200 ring-1 ring-brand-500/30 hover:bg-brand-500/25 disabled:opacity-50"
                >
                  {busy ? <Loader2 className="h-3 w-3 animate-spin" /> : <RotateCcw className="h-3 w-3" />}
                  {t("downloads.retry")}
                </button>
              )}
              {failed && !!job.addonId && <OtherLinkMenu job={job} />}
              {job.phase === "completed" && job.installPath && (
                <button
                  onClick={() => api.openLocalPath(job.installPath!)}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-brand-300 hover:bg-sf-raised hover:text-brand-200"
                >
                  <FolderOpen className="h-3 w-3" />
                  {t("downloads.open_installed_folder")}
                </button>
              )}
              {job.phase === "completed" && job.methodKind !== "torrent" && /^https?:/.test(job.url) && (
                <button
                  onClick={() => api.openExternal(job.url)}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                >
                  <ExternalLink className="h-3 w-3" />
                  {t("downloads.reopen_link")}
                </button>
              )}
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {canPause && (
            <IconButton title={t("downloads.pause")} onClick={() => pause(job.id)} tone="amber">
              <Pause className="h-3.5 w-3.5" />
            </IconButton>
          )}
          {canResume && (
            <IconButton title={t("downloads.resume")} onClick={() => resume(job.id)} tone="brand">
              <Play className="h-3.5 w-3.5" />
            </IconButton>
          )}
          {!terminal && (
            <IconButton title={t("common.cancel")} onClick={() => cancel(job.id)}>
              <XCircle className="h-3.5 w-3.5" />
            </IconButton>
          )}
          {terminal && (
            <IconButton title={t("common.remove_from_list")} onClick={() => clear(job.id)} tone="rose">
              <Trash2 className="h-3.5 w-3.5" />
            </IconButton>
          )}
        </div>
      </div>
    </motion.li>
  );
}

function IconButton({
  title,
  onClick,
  tone,
  children,
}: {
  title: string;
  onClick: () => void;
  tone?: "amber" | "brand" | "rose";
  children: React.ReactNode;
}) {
  const hover =
    tone === "amber"
      ? "hover:border-amber-500/40 hover:text-amber-300"
      : tone === "brand"
        ? "hover:border-brand-500/40 hover:text-brand-300"
        : tone === "rose"
          ? "hover:border-rose-500/40 hover:text-rose-300"
          : "hover:border-sf-border hover:text-sf-text";
  return (
    <button
      onClick={onClick}
      title={title}
      aria-label={title}
      className={`rounded-lg border border-transparent p-1.5 text-sf-muted transition-colors hover:bg-sf-raised ${hover}`}
    >
      {children}
    </button>
  );
}

/** (25/10) «Otro enlace»: los demás métodos del addon (ModsFire, torrent…). */
function OtherLinkMenu({ job }: { job: DownloadJob }) {
  const addonOf = useDownloadsStore((s) => s.addonOf);
  const startWith = useDownloadsStore((s) => s.startWith);
  const [open, setOpen] = useState(false);
  const [addon, setAddon] = useState<Addon | null | undefined>(undefined);
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);
  const toggle = () => {
    setOpen((v) => !v);
    if (addon === undefined) void addonOf(job).then(setAddon);
  };
  const others: DownloadMethod[] = (addon?.downloadMethods ?? []).filter(
    (m) => !(m.name === job.methodName && m.url === job.url),
  );
  return (
    <div ref={ref} className="relative">
      <button
        onClick={toggle}
        className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-sf-muted ring-1 ring-sf-border hover:bg-sf-raised hover:text-sf-text"
      >
        <Link2 className="h-3 w-3" />
        {t("downloads.other_link")}
        <ChevronDown className="h-3 w-3" />
      </button>
      {open && (
        <div className="absolute left-0 top-full z-10 mt-1 w-60 overflow-hidden rounded-lg border border-sf-border bg-sf-surface py-1 shadow-xl shadow-black/40">
          {addon === undefined ? (
            <div className="flex items-center gap-2 px-3 py-2 text-[11px] text-sf-muted">
              <Loader2 className="h-3 w-3 animate-spin" />
              {t("downloads.other_link_loading")}
            </div>
          ) : others.length === 0 || !addon ? (
            <div className="px-3 py-2 text-[11px] text-sf-muted">{t("downloads.other_link_none")}</div>
          ) : (
            others.map((m) => (
              <button
                key={`${m.name}-${m.url}`}
                onClick={() => {
                  setOpen(false);
                  void startWith(job, addon, m);
                }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-sf-text hover:bg-sf-raised"
              >
                <Download className="h-3 w-3 shrink-0 text-brand-300" />
                <span className="truncate">{m.name}</span>
                <span className="ml-auto shrink-0 text-[10px] uppercase text-sf-muted">{m.kind}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
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
      className="rounded-xl border border-sf-border bg-sf-surface p-3"
    >
      <div className="flex items-start gap-3">
        <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-brand-300" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-sf-text">
            {item.title}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-sf-muted">
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
          <div className="mt-1 truncate font-mono text-[11px] text-sf-muted">
            {item.installPath}
          </div>
          <div className="mt-2 flex items-center gap-3">
            <button
              onClick={() => api.openLocalPath(item.installPath)}
              className="inline-flex items-center gap-1 text-[11px] text-brand-300 hover:text-brand-200"
            >
              <FolderOpen className="h-3 w-3" />
              {t("pkg.open_folder")}
            </button>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => forget(item.id)}
            className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-red-300"
            title={t("downloads.forget_history.tooltip")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </motion.li>
  );
}

function PhaseIcon({ phase }: { phase: DownloadJob["phase"] }) {
  const common = "h-3.5 w-3.5 shrink-0";
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
      return <XCircle className={`${common} text-sf-muted`} />;
    case "error":
      return <AlertCircle className={`${common} text-red-400`} />;
  }
}

function ProgressBar({ job }: { job: DownloadJob }) {
  const total = job.bytesTotal;
  const pct = total > 0 ? Math.min(100, (job.bytesDone / total) * 100) : 0;
  const color = job.phase === "paused" ? "bg-amber-400" : "bg-gradient-to-r from-brand-500 to-brand-300";
  return (
    <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-sf-raised">
      {total > 0 ? (
        <div className={`h-full rounded-full ${color} transition-[width] duration-300`} style={{ width: `${pct}%` }} />
      ) : (
        // Sin tamaño todavía (esperando al servidor / instalando): barra
        // indeterminada.
        <div className={`h-full w-1/3 animate-pulse rounded-full ${color}`} />
      )}
    </div>
  );
}

/**
 * Compact "123.4 MB / 789.0 MB · 2.3 MB/s · 45s" line under the progress
 * bar. Kept tiny so multiple concurrent jobs can stack cleanly in the
 * side panel.
 */
function ProgressDetails({ job }: { job: DownloadJob }) {
  const hasTotal = job.bytesTotal > 0;
  const pct = hasTotal ? `${((job.bytesDone / job.bytesTotal) * 100).toFixed(1)}%` : "";
  let left: string;
  if (job.phase === "paused") {
    left = t("downloads.paused_at", { pct: pct || formatBytes(job.bytesDone) });
  } else if (job.phase === "installing" || !hasTotal) {
    left = job.message ? translateJobMessage(job.message) : t("downloads.waiting");
  } else {
    const parts = [`${formatBytes(job.bytesDone)} / ${formatBytes(job.bytesTotal)}`];
    if (job.speedBps > 0) parts.push(`${formatBytes(job.speedBps)}/s`);
    if (job.etaSeconds !== null && job.etaSeconds > 0) parts.push(t("downloads.eta_remaining", { time: formatDuration(job.etaSeconds) }));
    // "Parte 1 de 2" y similares.
    if (job.message) parts.push(translateJobMessage(job.message));
    left = parts.join(" · ");
  }
  return (
    <div className="mt-1 flex items-center justify-between gap-2 text-[11px] text-sf-muted">
      <span className="truncate" title={job.resumable ? t("downloads.resumable_hint") : undefined}>{left}</span>
      {job.phase === "downloading" && pct && <span className="shrink-0 tabular-nums text-sf-text/80">{pct}</span>}
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
 * Converts the SQLite TEXT timestamp ("YYYY-MM-DD HH:MM:SS" in UTC)
 * to a friendly relative label. Labels are i18n-driven so they switch
 * with the active language (ES: «hace 5m» · EN: "5m ago").
 */
function formatRelativeDate(iso: string): string {
  // SQLite emits "YYYY-MM-DD HH:MM:SS" without 'T' or trailing 'Z'.
  // Some browsers parse it as local time — normalize to UTC ISO-8601.
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const ts = Date.parse(normalized);
  if (Number.isNaN(ts)) return iso;
  const diffMs = Date.now() - ts;
  if (diffMs < 0) return t("downloads.rel.now");
  const sec = Math.floor(diffMs / 1000);
  if (sec < 60) return t("downloads.rel.few_secs");
  const min = Math.floor(sec / 60);
  if (min < 60) return t("downloads.rel.minutes", { n: String(min) });
  const hr = Math.floor(min / 60);
  if (hr < 24) return t("downloads.rel.hours", { n: String(hr) });
  const day = Math.floor(hr / 24);
  if (day < 30) return t("downloads.rel.days", { n: String(day) });
  const month = Math.floor(day / 30);
  if (month < 12) return t("downloads.rel.months", { n: String(month) });
  const year = Math.floor(day / 365);
  return t("downloads.rel.years", { n: String(year) });
}

/**
 * Re-translate Rust-backend `job.message` strings to the active
 * locale. The backend emits literal Spanish like "Se instalaron N
 * paquete(s) (X.X MB)" — we pattern-match and reformat via i18n.
 * Falls through unchanged for messages we don't recognize.
 */
function translateJobMessage(msg: string): string {
  // "Se instalaron N paquete(s) (X.X MB)" → i18n
  const installedMatch = msg.match(/^Se instalaron\s+(\d+)\s+paquetes?\s+\(([\d.]+)\s*MB\)/);
  if (installedMatch) {
    const count = installedMatch[1];
    const key = count === "1" ? "downloads.msg.installed.one" : "downloads.msg.installed.many";
    return t(key, { count, mb: installedMatch[2] });
  }
  // (v7.1.1) `job.message` se PERSISTE en la DB en el idioma en que se generó.
  // Un download abierto cuando el backend estaba en español queda con el texto
  // español guardado, y sigue saliendo así aunque la app esté en inglés. Los
  // re-mapeamos al estado localizado actual por su contenido conocido.
  if (
    msg.startsWith("Descarga abierta dentro de SimFleet") ||
    msg.startsWith("Abierto en el navegador") ||
    msg.includes("Instalar desde archivo")
  ) {
    return t("downloads.msg.opened_in_browser");
  }
  // "Abriendo la descarga…" (fase inicial, español persistido)
  if (msg.startsWith("Abriendo la descarga") || msg.startsWith("Extrayendo archivo")) {
    return t("downloads.msg.opened_in_browser");
  }
  return msg;
}
