import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, History, Loader2, Lock, RefreshCcw } from "lucide-react";
import { api } from "../lib/tauri";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import type { ContentChange, ContentPackage, ContentReport } from "../lib/types";
import { ToggleSwitch } from "./AddonToggle";

const PAGE = 100;
const KINDS = [
  "aircraft",
  "airport",
  "scenery",
  "traffic",
  "discovery",
  "challenge",
  "training",
  "bush",
  "activity",
  "character",
  "library",
  "other",
];
const FIELD =
  "rounded-lg border border-sf-border bg-sf-bg px-2.5 py-1.5 text-xs text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500";
const BTN =
  "inline-flex items-center gap-1 rounded-lg border border-sf-border bg-sf-raised px-2.5 py-1 font-semibold text-sf-text hover:border-brand-500/50 disabled:cursor-not-allowed disabled:opacity-50";

type Status = "all" | "on" | "off" | "changed";

function errorText(e: unknown): string {
  const code = String(e).replace(/^Error:\s*/, "");
  const known = ["sim_running", "test_only", "changed", "locked", "not_found", "unknown_format", "write_check"];
  return known.includes(code) ? t(`content.err.${code}`) : code;
}

function backupDate(iso: string | null): string {
  if (!iso) return "—";
  return new Date(iso).toLocaleString(getActiveLocale(), { dateStyle: "medium", timeStyle: "medium" });
}

function PackageRow({
  pkg,
  on,
  changed,
  disabled,
  onToggle,
}: {
  pkg: ContentPackage;
  on: boolean;
  changed: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <li className={cn("flex items-center gap-2 rounded-lg px-2 py-1", changed && "bg-brand-500/10")}>
      <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-sf-text" title={pkg.name}>
        {pkg.name}
      </span>
      <span className="shrink-0 text-[10px] text-sf-muted">{t(`content.kind.${pkg.kind}`)}</span>
      {pkg.locked ? (
        <span title={t(`content.locked.${pkg.locked}`)} className="flex w-8 shrink-0 justify-center text-sf-muted">
          <Lock className="h-3.5 w-3.5" />
        </span>
      ) : (
        <ToggleSwitch on={on} busy={disabled} onToggle={onToggle} small />
      )}
    </li>
  );
}

/**
 * (07/11) Etapa F — apaga contenido de fábrica de MSFS 2024 en `Content.xml`.
 * Se marca, se revisa y se aplica de una vez: antes de escribir hay copia con
 * fecha, y con MSFS abierto no se puede.
 */
export function ContentXmlPanel() {
  const [report, setReport] = useState<ContentReport | null>(null);
  const [user, setUser] = useState<string | null>(null);
  const [marked, setMarked] = useState<Map<string, boolean>>(new Map());
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState("all");
  const [status, setStatus] = useState<Status>("all");
  const [limit, setLimit] = useState(PAGE);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [sure, setSure] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [showBackups, setShowBackups] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async (who: string | null) => {
    setLoading(true);
    try {
      setReport(await api.contentReport(who));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load(user);
  }, [load, user]);

  const counts = useMemo(() => {
    const packages = report?.packages ?? [];
    return {
      on: packages.filter((p) => p.active).length,
      off: packages.filter((p) => !p.active && p.locked !== "system").length,
      system: packages.filter((p) => p.locked === "system").length,
    };
  }, [report]);

  const isOn = (p: ContentPackage) => marked.get(p.name) ?? p.active;
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (report?.packages ?? []).filter((p) => {
      if (q && !p.name.toLowerCase().includes(q)) return false;
      if (kind !== "all" && p.kind !== kind) return false;
      if (status === "changed") return marked.has(p.name);
      if (status === "on") return marked.get(p.name) ?? p.active;
      if (status === "off") return !(marked.get(p.name) ?? p.active);
      return true;
    });
  }, [report, query, kind, status, marked]);
  useEffect(() => setLimit(PAGE), [query, kind, status]);

  const toggle = (p: ContentPackage) => {
    setSure(false);
    setNotice(null);
    setMarked((current) => {
      const next = new Map(current);
      const wanted = !(current.get(p.name) ?? p.active);
      if (wanted === p.active) next.delete(p.name);
      else next.set(p.name, wanted);
      return next;
    });
  };

  const changes: ContentChange[] = useMemo(
    () => [...marked].map(([name, active]) => ({ name, active })),
    [marked],
  );
  const turningOff = changes.filter((c) => !c.active).length;
  const touchesLibrary = (report?.packages ?? []).some((p) => p.kind === "library" && marked.get(p.name) === false);

  // Sin marcas, el filtro «Solo los marcados» dejaría la lista vacía.
  const clearMarks = () => {
    setMarked(new Map());
    setStatus((s) => (s === "changed" ? "all" : s));
  };

  const apply = async (list: ContentChange[]) => {
    if (!report?.user || !report.stamp) return;
    setBusy(true);
    setError(null);
    try {
      const n = await api.contentApply(report.user, report.stamp, list);
      clearMarks();
      setNotice(t("content.applied", { n: String(n) }));
      await load(report.user);
    } catch (e) {
      setError(errorText(e));
      await load(report.user);
    } finally {
      setBusy(false);
      setSure(false);
    }
  };

  const restore = async (file: string) => {
    if (!report?.user) return;
    setBusy(true);
    setError(null);
    try {
      await api.contentRestore(report.user, file);
      clearMarks();
      setNotice(t("content.restored"));
      await load(report.user);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      setRestoring(null);
    }
  };

  if (!report || !report.user) {
    return (
      <div className="flex items-center gap-2 rounded-xl border border-sf-border bg-sf-surface px-4 py-3 text-xs text-sf-muted">
        {loading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
        {error ?? (loading ? t("content.loading") : t("content.none"))}
      </div>
    );
  }

  const blocked = report.simRunning || busy || report.unknownFormat;
  return (
    <div className="space-y-3 rounded-xl border border-sf-border bg-sf-surface px-4 py-3 text-xs">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-semibold text-sf-text">
            {t("content.summary", {
              on: counts.on.toLocaleString(),
              off: counts.off.toLocaleString(),
              system: counts.system.toLocaleString(),
            })}
          </p>
          <p className="text-[11px] text-sf-muted">{t("content.intro", { n: String(report.community) })}</p>
        </div>
        <button
          onClick={() => {
            setNotice(null);
            void load(report.user);
          }}
          disabled={loading}
          title={t("content.reload")}
          aria-label={t("content.reload")}
          className="shrink-0 rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text disabled:opacity-50"
        >
          <RefreshCcw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
        </button>
      </div>

      {report.users.length > 1 && (
        <label className="flex items-center gap-2 text-[11px] text-sf-muted">
          {t("content.user")}
          <select
            value={report.user}
            onChange={(e) => {
              clearMarks();
              setNotice(null);
              setUser(e.target.value);
            }}
            className={FIELD}
          >
            {report.users.map((u) => (
              <option key={u} value={u}>
                {u}
              </option>
            ))}
          </select>
        </label>
      )}

      {report.simRunning && (
        <p className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200">
          <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
          {t("content.sim_running")}
        </p>
      )}
      {report.unknownFormat && (
        <p className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200">
          <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
          {t("content.err.unknown_format")}
        </p>
      )}
      {report.drift.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-brand-500/40 bg-brand-500/10 px-3 py-2 text-[11px] text-brand-200">
          <span className="min-w-0 flex-1">{t("content.drift", { n: String(report.drift.length) })}</span>
          <button type="button" className={BTN} disabled={blocked} onClick={() => void apply(report.drift)}>
            {t("content.drift_apply")}
          </button>
        </div>
      )}
      {notice && <p className="text-[11px] text-emerald-300">{notice}</p>}
      {error && <p className="text-[11px] text-rose-300">{error}</p>}

      {!report.unknownFormat && (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("content.search")}
              aria-label={t("content.search")}
              className={cn(FIELD, "min-w-[12rem] flex-1")}
            />
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value)}
              aria-label={t("content.kind")}
              className={FIELD}
            >
              <option value="all">{t("content.kind.all")}</option>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {t(`content.kind.${k}`)}
                </option>
              ))}
            </select>
            <select
              value={status}
              onChange={(e) => setStatus(e.target.value as Status)}
              aria-label={t("content.status")}
              className={FIELD}
            >
              {(["all", "on", "off", "changed"] as const).map((s) => (
                <option key={s} value={s}>
                  {t(`content.status.${s}`)}
                </option>
              ))}
            </select>
          </div>
          {kind === "library" && <p className="text-[11px] text-amber-300">{t("content.library_note")}</p>}

          <ul className="max-h-72 space-y-0.5 overflow-y-auto">
            {rows.slice(0, limit).map((p) => (
              <PackageRow
                key={p.name}
                pkg={p}
                on={isOn(p)}
                changed={marked.has(p.name)}
                disabled={blocked}
                onToggle={() => toggle(p)}
              />
            ))}
            {rows.length === 0 && <li className="px-2 py-1 text-[11px] text-sf-muted">{t("content.empty")}</li>}
          </ul>
          {rows.length > limit && (
            <button type="button" className={BTN} onClick={() => setLimit((n) => n + PAGE)}>
              {t("content.more", { n: String(rows.length - limit) })}
            </button>
          )}

          {changes.length > 0 && (
            <div className="space-y-2 rounded-lg border border-sf-border bg-sf-bg/40 px-3 py-2">
              <p className="text-[11px] text-sf-text">
                {t("content.pending", { off: String(turningOff), on: String(changes.length - turningOff) })}
              </p>
              {touchesLibrary && <p className="text-[11px] text-amber-300">{t("content.library_note")}</p>}
              {sure ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="min-w-0 flex-1 text-[11px] text-sf-muted">{t("content.confirm")}</span>
                  <button
                    type="button"
                    onClick={() => void apply(changes)}
                    disabled={blocked}
                    className="inline-flex items-center gap-1 rounded-lg bg-brand-500 px-2.5 py-1 font-semibold text-white hover:bg-brand-400 disabled:opacity-50"
                  >
                    {busy && <Loader2 className="h-3 w-3 animate-spin" />}
                    {t("content.confirm_yes")}
                  </button>
                  <button type="button" onClick={() => setSure(false)} className={BTN}>
                    {t("content.keep")}
                  </button>
                </div>
              ) : (
                <div className="flex flex-wrap items-center gap-2">
                  <button type="button" onClick={() => setSure(true)} disabled={blocked} className={BTN}>
                    {t("content.apply")}
                  </button>
                  <button type="button" onClick={clearMarks} className={BTN}>
                    {t("content.discard")}
                  </button>
                </div>
              )}
            </div>
          )}
        </>
      )}

      {report.backups.length > 0 && (
        <div className="space-y-1.5">
          <button
            type="button"
            onClick={() => setShowBackups((v) => !v)}
            className="inline-flex items-center gap-1.5 text-[11px] font-medium text-sf-muted hover:text-sf-text"
          >
            <History className="h-3.5 w-3.5" />
            {t("content.backups", { n: String(report.backups.length) })}
          </button>
          {showBackups && (
            <ul className="space-y-1">
              {report.backups.map((b) => (
                <li key={b.file} className="flex flex-wrap items-center gap-2 rounded-lg bg-sf-bg/40 px-2 py-1">
                  <span className="min-w-0 flex-1 text-[11px] text-sf-text">{backupDate(b.savedAt)}</span>
                  <span className="text-[11px] text-sf-muted">{t("content.backup_off", { n: String(b.disabled) })}</span>
                  {restoring === b.file ? (
                    <>
                      <button type="button" className={BTN} disabled={blocked} onClick={() => void restore(b.file)}>
                        {t("content.restore_yes")}
                      </button>
                      <button type="button" className={BTN} onClick={() => setRestoring(null)}>
                        {t("content.keep")}
                      </button>
                    </>
                  ) : (
                    <button type="button" className={BTN} disabled={blocked} onClick={() => setRestoring(b.file)}>
                      {t("content.restore")}
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
