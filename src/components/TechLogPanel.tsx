import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Armchair,
  CheckCircle2,
  ClipboardCheck,
  Clock,
  Droplets,
  ExternalLink,
  Gauge,
  Loader2,
  Play,
  RefreshCw,
  ShieldAlert,
  Wrench,
  X,
} from "lucide-react";
import type {
  FlightLogEntry,
  TechLogActionRequest,
  TechLogAircraft,
  TechLogCabin,
  TechLogCabinLights,
  TechLogReadings,
  TechLogFlight,
  TechLogPendingOrder,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useTechLogStore } from "../stores/useTechLogStore";
import { useToastStore } from "../stores/useToastStore";

/**
 * (19/10) Etapa 2 — Mantenimiento de TechLog dentro de SimFleet.
 *
 * Para los aviones que tienen Fenix/PMDG TechLog, TechLog es la fuente de
 * verdad: aquí se ve su estado (GO / NO-GO, defectos, piezas gastadas,
 * consumibles y checks) y, si TechLog está abierto con ESE avión activo, se
 * puede actuar (reparar, diferir, reemplazar, servicio, check). Lo que se
 * hace se cobra en Finanzas.
 */

export function useTechLogAircraft(
  registration: string | null | undefined,
  managed = true,
  title?: string | null,
) {
  const epoch = useTechLogStore((s) => s.epoch);
  const [data, setData] = useState<TechLogAircraft | null>(null);
  const [loading, setLoading] = useState(false);
  const reload = useCallback(async () => {
    if (!registration) {
      setData(null);
      return;
    }
    setLoading(true);
    try {
      // (19/10) Etapa 3: sin Fenix/PMDG TechLog → Tech Log de SimFleet.
      setData(managed ? await api.techlogAircraft(registration) : await api.mxAircraft(registration, title));
    } catch {
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [registration, managed, title]);
  useEffect(() => {
    void reload();
  }, [reload, epoch]);
  return { data, loading, reload };
}

/** (19/10) Texto de estado según la fuente (TechLog o Tech Log de SimFleet). */
function statusText(data: TechLogAircraft): string {
  if (data.source === "simfleet") return t(data.active ? "mx.status" : "mx.in_flight");
  return t(data.active ? "techlog.live_active" : data.running ? "techlog.live_other" : "techlog.offline");
}

/** (20/10) Errores del backend → texto para el usuario. */
function actionError(msg: string): string {
  if (msg.includes("in_flight")) return t("mx.in_flight");
  if (msg.includes("not_installed")) return t("techlog.err.not_installed");
  if (msg.includes("launch_timeout")) return t("techlog.err.launch_timeout");
  if (msg.includes("no_airframe")) return t("techlog.err.no_airframe");
  if (msg.includes("not_active")) return t("techlog.not_active");
  return msg;
}

function dispatchStyle(status: string) {
  if (status === "GO")
    return { cls: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/40", label: "GO" };
  if (status === "GO_MEL")
    return { cls: "bg-amber-500/15 text-amber-300 ring-amber-500/40", label: "GO · MEL" };
  return { cls: "bg-rose-500/15 text-rose-300 ring-rose-500/40", label: "NO-GO" };
}

function fmtDate(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso.length === 10 ? `${iso}T00:00:00` : iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString(undefined, { day: "2-digit", month: "short", year: "numeric" });
}

function barColor(pct: number) {
  return pct >= 60 ? "bg-emerald-400" : pct >= 30 ? "bg-lime-400" : pct >= 10 ? "bg-amber-400" : "bg-rose-400";
}

/** Chip con la app TechLog y el estado de la conexión. */
export function TechLogBadge({ data }: { data: TechLogAircraft }) {
  const tone = data.active
    ? "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30"
    : data.running
      ? "bg-amber-500/15 text-amber-300 ring-amber-500/30"
      : "bg-sf-raised text-sf-muted ring-sf-border";
  return (
    <span
      className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ring-1", tone)}
      title={statusText(data)}
    >
      <span
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          data.active ? "bg-emerald-400" : data.running ? "bg-amber-400" : "bg-slate-400",
        )}
      />
      {data.appName}
    </span>
  );
}

/** Tarjeta resumen (pestaña Resumen del Hangar). */
export function TechLogSummaryCard({
  data,
  onOpen,
}: {
  data: TechLogAircraft;
  onOpen: () => void;
}) {
  const d = dispatchStyle(data.dispatch.status);
  const open = data.defects.filter((x) => x.status === "Open").length;
  const deferred = data.defects.filter((x) => x.status === "Deferred").length;
  return (
    <div className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-sf-muted">{t("techlog.summary.title")}</span>
        <TechLogBadge data={data} />
      </div>
      <div className="flex items-center gap-3">
        <span className={cn("rounded-lg px-3 py-1.5 font-mono text-lg font-bold ring-1", d.cls)}>{d.label}</span>
        <div className="grid flex-1 grid-cols-3 gap-2 text-center">
          <Mini label={t("techlog.summary.open")} value={open} tone={open ? "text-rose-300" : "text-sf-text"} />
          <Mini label={t("techlog.summary.deferred")} value={deferred} tone={deferred ? "text-amber-300" : "text-sf-text"} />
          <Mini
            label={t("techlog.summary.worn")}
            value={data.condition.low + data.condition.failed}
            tone={data.condition.failed ? "text-rose-300" : data.condition.low ? "text-amber-300" : "text-sf-text"}
          />
        </div>
      </div>
      {data.parts.length > 0 && (
        <ul className="mt-3 space-y-1.5">
          {data.parts.slice(0, 3).map((p) => (
            <li key={p.id} className="flex items-center gap-2 text-xs">
              <span className="min-w-0 flex-1 truncate text-sf-text">{p.name}</span>
              <div className="h-1.5 w-20 overflow-hidden rounded-full bg-sf-raised">
                <div className={cn("h-full rounded-full", barColor(p.remainingPct))} style={{ width: `${Math.max(2, p.remainingPct)}%` }} />
              </div>
              <span className="w-10 text-right tabular-nums text-sf-muted">{Math.round(p.remainingPct)}%</span>
            </li>
          ))}
        </ul>
      )}
      <button
        onClick={onOpen}
        className="mt-3 inline-flex items-center gap-1.5 text-xs font-medium text-brand-300 hover:text-brand-200"
      >
        <Wrench className="h-3.5 w-3.5" />
        {t("techlog.summary.open_panel")}
      </button>
    </div>
  );
}

function Mini({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="rounded-lg bg-sf-raised/60 px-2 py-1.5">
      <div className={cn("text-base font-semibold tabular-nums", tone)}>{value}</div>
      <div className="text-[10px] text-sf-muted">{label}</div>
    </div>
  );
}

/** Panel completo (pestaña Mantenimiento del Hangar). */
export function TechLogPanel({
  data,
  onReload,
  loading,
}: {
  data: TechLogAircraft;
  onReload: () => void;
  loading: boolean;
}) {
  const pushToast = useToastStore((s) => s.push);
  const epoch = useTechLogStore((s) => s.epoch);
  const [busy, setBusy] = useState<string | null>(null);
  const d = dispatchStyle(data.dispatch.status);
  const own = data.source === "simfleet";
  // (20/10) Con Fenix/PMDG TechLog se puede ordenar siempre: si está cerrado
  // SimFleet lo abre, y si no deja elegir el avión la orden queda en cola.
  const can = own ? data.active : true;

  // (20/10) Órdenes en cola de este avión.
  const [pending, setPending] = useState<TechLogPendingOrder[]>([]);
  const loadPending = useCallback(async () => {
    if (own) return setPending([]);
    try {
      setPending(await api.techlogPending(data.registration));
    } catch {
      setPending([]);
    }
  }, [own, data.registration]);
  useEffect(() => {
    void loadPending();
  }, [loadPending, epoch]);

  const run = async (busyKey: string, req: Omit<TechLogActionRequest, "registration">) => {
    setBusy(busyKey);
    try {
      const request = { registration: data.registration, ...req };
      const r = own ? await api.mxAction(request) : await api.techlogAction(request);
      if (r.queued) {
        pushToast({ kind: "info", title: t("techlog.queued.title"), message: waitText(r.message) });
      } else {
        pushToast({
          kind: r.ok ? "success" : "error",
          title: r.ok ? t("techlog.action.done") : t("techlog.action.refused"),
          message: r.message || undefined,
        });
      }
      onReload();
      void loadPending();
    } catch (e) {
      pushToast({ kind: "error", title: t("techlog.action.refused"), message: actionError(String(e)) });
    } finally {
      setBusy(null);
    }
  };

  /** (24/10) Por qué espera una orden (código del backend). */
  const waitText = (code?: string | null) =>
    code === "sim_closed" || code === "not_loaded" || code === "not_parked"
      ? t(`techlog.wait.${code}`)
      : t("techlog.queued.detail");

  const cancelPending = async (id: number) => {
    try {
      await api.techlogCancelPending(id);
    } finally {
      void loadPending();
    }
  };

  const [launching, setLaunching] = useState(false);
  const launch = async () => {
    if (own) return;
    setLaunching(true);
    try {
      await api.techlogLaunch(data.source);
      pushToast({ kind: "success", title: t("techlog.launched", { app: data.appName }) });
      onReload();
    } catch (e) {
      pushToast({ kind: "error", title: t("techlog.action.refused"), message: actionError(String(e)) });
    } finally {
      setLaunching(false);
    }
  };

  const ActionBtn = ({
    k,
    label,
    req,
    what,
    tone = "neutral",
  }: {
    k: string;
    label: string;
    req: Omit<TechLogActionRequest, "registration">;
    /** Sobre qué se actúa (para el texto de la orden si queda en cola). */
    what?: string;
    tone?: "neutral" | "primary";
  }) => (
    <button
      disabled={!can || busy !== null}
      onClick={() => void run(k, { ...req, label: what ? `${label} · ${what}` : label })}
      title={can ? undefined : t("mx.in_flight")}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40",
        tone === "primary"
          ? "border-brand-500/50 bg-brand-500/15 text-brand-200 hover:bg-brand-500/25"
          : "border-sf-border bg-sf-raised text-sf-text hover:border-brand-500/40",
      )}
    >
      {busy === k && <Loader2 className="h-3 w-3 animate-spin" />}
      {label}
    </button>
  );

  return (
    <div className="space-y-4">
      {/* Cabecera: app, conexión y despacho. */}
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-sf-border bg-sf-bg/40 p-4">
        <span className={cn("rounded-lg px-3 py-1.5 font-mono text-xl font-bold ring-1", d.cls)}>{d.label}</span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <TechLogBadge data={data} />
            <span className="text-xs text-sf-muted">
              {data.variant} {data.engine} · {data.flightHours.toLocaleString()} h · {Math.round(data.cycles).toLocaleString()} {t("techlog.cycles")}
              {data.egtMarginC.length > 0 && ` · EGT ${data.egtMarginC.map((m) => `${Math.round(m)}°`).join(" / ")}`}
            </span>
          </div>
          <p className="mt-1 text-[11px] text-sf-muted">
            {statusText(data)}
          </p>
          {data.dispatch.reasons.length > 0 && (
            <ul className="mt-1.5 space-y-0.5 text-[11px] text-sf-text/90">
              {data.dispatch.reasons.slice(0, 4).map((r) => (
                <li key={r}>• {r}</li>
              ))}
            </ul>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <button
            onClick={onReload}
            title={t("techlog.reload")}
            className="rounded-md border border-sf-border bg-sf-raised p-1.5 text-sf-muted hover:text-sf-text"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
          </button>
          {/* (20/10) En MSFS 2020 no hay EFB: la tablet se abre en el navegador. */}
          {data.running && data.tabletUrl && (
            <button
              onClick={() => void api.openExternal(data.tabletUrl)}
              className="inline-flex items-center gap-1 rounded-md border border-sf-border bg-sf-raised px-2 py-1.5 text-[11px] text-sf-text hover:border-brand-500/40"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              {t("techlog.open_tablet")}
            </button>
          )}
          {!own && !data.running && (
            <button
              onClick={() => void launch()}
              disabled={launching}
              className="inline-flex items-center gap-1 rounded-md border border-sf-border bg-sf-raised px-2 py-1.5 text-[11px] text-sf-text hover:border-brand-500/40 disabled:opacity-50"
            >
              {launching ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
              {t("techlog.launch")}
            </button>
          )}
        </div>
      </div>

      {!can && (
        <p className="rounded-lg border border-sf-border bg-sf-raised/40 px-3 py-2 text-[11px] text-sf-muted">
          {t("mx.in_flight")}
        </p>
      )}
      {/* (24/10) Las órdenes sólo salen con este avión cargado y estacionado. */}
      {!own && data.orderWait && (
        <p className="rounded-lg border border-sf-border bg-sf-raised/40 px-3 py-2 text-[11px] text-sf-muted">
          {t("techlog.wait.hint", { app: data.appName, why: waitText(data.orderWait) })}
        </p>
      )}
      {!own && data.pendingWork && (
        <p className="flex items-start gap-2 rounded-lg border border-brand-500/40 bg-brand-500/10 px-3 py-2 text-[11px] text-brand-200">
          <Clock className="mt-px h-3.5 w-3.5 shrink-0" />
          {t("techlog.pending_work", { app: data.appName, work: data.pendingWork })}
        </p>
      )}

      {/* (20/10) Órdenes en cola para TechLog. */}
      {pending.length > 0 && (
        <Section
          icon={<Clock className="h-4 w-4 text-amber-300" />}
          title={t("techlog.pending.title", { n: pending.length })}
        >
          <p className="mb-1 text-[11px] text-sf-muted">{t("techlog.pending.hint")}</p>
          <ul className="divide-y divide-sf-border/50">
            {pending.map((o) => (
              <li key={o.id} className="flex items-center gap-2 py-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-medium text-sf-text">{o.label ?? o.action}</div>
                  <div className="text-[10px] text-sf-muted">{fmtDate(o.createdAt)}</div>
                </div>
                <button
                  onClick={() => void cancelPending(o.id)}
                  title={t("techlog.pending.cancel")}
                  className="shrink-0 rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {/* Defectos abiertos y diferidos. */}
      <Section
        icon={<ShieldAlert className="h-4 w-4 text-rose-300" />}
        title={t("techlog.defects.title", { n: data.defects.length })}
        action={
          data.defects.some((x) => x.status === "Open") ? (
            <ActionBtn k="callMaintenance" label={t("techlog.call_maintenance")} req={{ action: "callMaintenance" }} tone="primary" />
          ) : undefined
        }
      >
        {data.defects.length === 0 ? (
          <Empty icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />} text={t("techlog.defects.none")} />
        ) : (
          <ul className="divide-y divide-sf-border/50">
            {data.defects.map((x) => (
              <li key={x.id} className="flex flex-wrap items-start gap-2 py-2">
                <span
                  className={cn(
                    "mt-0.5 shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-bold",
                    x.category === "NoGo"
                      ? "bg-rose-500/20 text-rose-300"
                      : x.status === "Deferred"
                        ? "bg-amber-500/20 text-amber-300"
                        : "bg-sf-raised text-sf-text",
                  )}
                >
                  {x.category === "NoGo" ? "NO-GO" : x.category}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-medium text-sf-text">
                    {x.title}
                    {x.melId && <span className="ml-1.5 font-mono text-[10px] text-sf-muted">MEL {x.melId}</span>}
                  </div>
                  {x.detail && <div className="mt-0.5 text-[11px] text-sf-muted">{x.detail}</div>}
                  <div className="mt-0.5 text-[10px] text-sf-muted">
                    {x.status === "Deferred"
                      ? t("techlog.defects.deferred_until", { d: fmtDate(x.deferredUntil) })
                      : t("techlog.defects.raised", { d: fmtDate(x.raisedUtc) })}
                    {x.restrictions && ` · ${x.restrictions}`}
                  </div>
                </div>
                <div className="flex shrink-0 gap-1">
                  <ActionBtn k={`fix-${x.id}`} label={t("techlog.fix")} req={{ action: "fix", id: x.id }} what={x.title} />
                  {x.status === "Open" && x.canDefer && x.latestDeferral && (
                    <ActionBtn
                      k={`defer-${x.id}`}
                      label={t("techlog.defer_until", { d: fmtDate(x.latestDeferral) })}
                      req={{ action: "defer", id: x.id, until: x.latestDeferral }}
                      what={x.title}
                    />
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      {/* Piezas gastadas / degradadas / falladas. */}
      <Section
        icon={<Wrench className="h-4 w-4 text-amber-300" />}
        title={t("techlog.parts.title", { n: data.parts.length })}
        subtitle={t("techlog.parts.condition", {
          good: data.condition.good,
          low: data.condition.low,
          failed: data.condition.failed,
        })}
      >
        {data.parts.length === 0 ? (
          <Empty icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />} text={t("techlog.parts.none")} />
        ) : (
          <ul className="divide-y divide-sf-border/50">
            {data.parts.map((p) => (
              <li key={p.id} className="flex flex-wrap items-center gap-2 py-2">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-xs font-medium text-sf-text">
                    {p.name}
                    {p.status !== "Ok" && (
                      <span className={cn("rounded px-1 text-[9px] font-bold uppercase", p.status === "Failed" ? "bg-rose-500/20 text-rose-300" : "bg-amber-500/20 text-amber-300")}>
                        {t(p.status === "Failed" ? "techlog.parts.failed" : "techlog.parts.degraded")}
                      </span>
                    )}
                  </div>
                  <div className="text-[10px] text-sf-muted">
                    ATA {p.ata} · {p.system}
                    {p.repairs > 0 && ` · ${t("techlog.parts.repairs", { n: p.repairs })}`}
                  </div>
                </div>
                <div className="flex w-32 items-center gap-1.5">
                  <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sf-raised">
                    <div className={cn("h-full rounded-full", barColor(p.remainingPct))} style={{ width: `${Math.max(2, p.remainingPct)}%` }} />
                  </div>
                  <span className="w-9 text-right text-[11px] tabular-nums text-sf-muted">{Math.round(p.remainingPct)}%</span>
                </div>
                <div className="flex shrink-0 gap-1">
                  {p.kind !== "wear" && p.status !== "Failed" && (
                    <ActionBtn k={`pfix-${p.id}`} label={t("techlog.repair")} req={{ action: "partFix", ids: [p.id] }} what={p.name} />
                  )}
                  <ActionBtn k={`prep-${p.id}`} label={t("techlog.replace")} req={{ action: "partReplace", ids: [p.id] }} what={p.name} />
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {/* Consumibles. */}
        <Section icon={<Droplets className="h-4 w-4 text-sky-300" />} title={t("techlog.consumables.title")}>
          <ul className="space-y-2">
            {data.consumables.map((c) => {
              const pct = c.capacity > 0 ? Math.min(100, (c.level / c.capacity) * 100) : 0;
              return (
                <li key={c.serviceKind} className="flex items-center gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center justify-between gap-2 text-xs">
                      <span className="truncate text-sf-text">{c.name}</span>
                      <span className={cn("tabular-nums", c.status === "below_min" ? "text-rose-300" : c.status === "low" ? "text-amber-300" : "text-sf-muted")}>
                        {c.level.toLocaleString()} / {c.capacity.toLocaleString()} {c.unit}
                      </span>
                    </div>
                    <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-sf-raised">
                      <div
                        className={cn("h-full rounded-full", c.status === "below_min" ? "bg-rose-400" : c.status === "low" ? "bg-amber-400" : "bg-sky-400")}
                        style={{ width: `${Math.max(2, pct)}%` }}
                      />
                    </div>
                  </div>
                  {/* (23/10) Fenix TechLog 2.2: el aceite lo lleva el Fenix; rellenar no hace nada. */}
                  {c.followsSim ? (
                    <span
                      title={t("techlog.consumables.follows_sim_hint")}
                      className="shrink-0 rounded-md border border-sf-border px-2 py-1 text-[10px] text-sf-muted"
                    >
                      {t("techlog.consumables.follows_sim")}
                    </span>
                  ) : (
                    <ActionBtn
                      k={`svc-${c.serviceKind}`}
                      label={c.fills ? t("techlog.empty") : t("techlog.fill")}
                      req={{ action: "service", key: c.serviceKind }}
                      what={c.name}
                    />
                  )}
                </li>
              );
            })}
          </ul>
        </Section>

        {/* Checks. */}
        <Section icon={<ClipboardCheck className="h-4 w-4 text-brand-300" />} title={t("techlog.checks.title")}>
          <ul className="divide-y divide-sf-border/50">
            {data.checks.map((c) => (
              <li key={c.key} className="flex items-center gap-2 py-2">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-xs font-medium text-sf-text">
                    {c.name}
                    {c.overdue && (
                      <span className="inline-flex items-center gap-0.5 rounded bg-rose-500/20 px-1 text-[9px] font-bold uppercase text-rose-300">
                        <AlertTriangle className="h-2.5 w-2.5" />
                        {t("techlog.checks.overdue")}
                      </span>
                    )}
                  </div>
                  <div className="text-[10px] text-sf-muted">
                    {t("techlog.checks.last", { d: fmtDate(c.lastDoneUtc) })}
                    {c.hoursLeft != null && !c.overdue && ` · ${t("techlog.checks.left", { h: Math.round(c.hoursLeft).toLocaleString() })}`}
                  </div>
                </div>
                <ActionBtn k={`chk-${c.key}`} label={t("techlog.checks.perform")} req={{ action: "check", key: c.key }} what={c.name} />
              </li>
            ))}
          </ul>
        </Section>
      </div>

      {/* (23/10) TechLog 2.1: cabina del Fenix cargado en el sim. */}
      {!own && data.running && data.active && <CabinSection registration={data.registration} />}
      {/* (23/10) TechLog 2.2: lecturas en vivo del Fenix. */}
      {!own && data.running && data.active && <ReadingsSection registration={data.registration} />}

      <p className="text-[10px] text-sf-muted">{t("techlog.charges_note")}</p>
    </div>
  );
}

/**
 * (23/10) Página "Fenix readings" de Fenix TechLog 2.2: aceite, motores,
 * hidráulica, eléctrico y aire del Fenix CARGADO, sólo lectura. Con TechLog
 * anterior a 2.2 no aparece. Se refresca cada 15 s.
 */
function ReadingsSection({ registration }: { registration: string }) {
  const [r, setR] = useState<TechLogReadings | null | undefined>(undefined);
  const load = useCallback(async () => {
    try {
      setR(await api.techlogReadings(registration));
    } catch {
      setR(null);
    }
  }, [registration]);
  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), 15_000);
    return () => window.clearInterval(id);
  }, [load]);
  if (r === null) return null;
  const fmt = (v: number | null, digits: number, unit: string) =>
    v === null
      ? "—"
      : `${v.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits })}${unit ? ` ${unit}` : ""}`;
  return (
    <Section
      icon={<Gauge className="h-4 w-4 text-brand-300" />}
      title={t("techlog.readings.title")}
      subtitle={t("techlog.readings.subtitle")}
    >
      {r === undefined ? (
        <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.cabin.loading")} />
      ) : !r.live ? (
        <Empty icon={<Clock className="h-4 w-4" />} text={t("techlog.readings.waiting")} />
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {r.groups.map((g) => (
            <div key={g.name} className="space-y-1">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{g.name}</div>
              {g.items.map((i) => (
                <div key={i.label} className="flex items-baseline justify-between gap-2 text-xs">
                  <span className="truncate text-sf-muted">{i.label}</span>
                  <span className="shrink-0 tabular-nums text-sf-text">{fmt(i.value, i.digits, i.unit)}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

const CABIN_LIGHTS: TechLogCabinLights[] = ["Bright", "Dim1", "Dim2", "Off"];
const WATER_STEPS = [0, 25, 50, 75, 100];
const CABIN_DOORS = ["1L", "1R", "2L", "2R", "3L", "3R", "4L", "4R"];

/**
 * (23/10) Página "Cabin" de Fenix TechLog 2.1 dentro del Hangar: agua y
 * residuos, puertas y toboganes, baños, temperatura, luces y señales del
 * Fenix CARGADO, más el pre-select de agua y las luces de cabina. TechLog la
 * refresca cada ~15 s; aquí igual. Con TechLog < 2.1 no aparece.
 */
function CabinSection({ registration }: { registration: string }) {
  const pushToast = useToastStore((s) => s.push);
  const [cab, setCab] = useState<TechLogCabin | null | undefined>(undefined);
  const [busy, setBusy] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      setCab(await api.techlogCabin(registration));
    } catch {
      setCab(null);
    }
  }, [registration]);
  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), 15_000);
    return () => window.clearInterval(id);
  }, [load]);
  if (cab === null) return null;

  const v = cab?.values ?? {};
  const has = (k: string) => v[k] !== undefined;
  const on = (k: string) => (v[k] ?? 0) > 0.5;
  const set = async (control: "lights" | "preselect", value: TechLogCabinLights | number) => {
    setBusy(`${control}-${value}`);
    try {
      const r = await api.techlogCabinSet(registration, control, value);
      pushToast({
        kind: r.ok ? "success" : "error",
        title: r.ok ? t("techlog.cabin.set_done") : t("techlog.cabin.set_refused"),
        message: r.message || undefined,
      });
      // El Fenix tarda unos segundos en reflejarlo.
      window.setTimeout(() => void load(), 2_000);
    } catch (e) {
      pushToast({ kind: "error", title: t("techlog.cabin.set_refused"), message: actionError(String(e)) });
    } finally {
      setBusy(null);
    }
  };

  const doors = CABIN_DOORS.filter((d) => has(`cabDoorD${d}`));
  const doorState = (d: string) =>
    on(`cabDoorD${d}`) ? "open" : has(`cabSlideD${d}`) && !on(`cabSlideD${d}`) ? "disarmed" : "ok";
  const overall = doors.some((d) => doorState(d) === "open")
    ? "open"
    : doors.some((d) => doorState(d) === "disarmed")
      ? "disarmed"
      : "ok";
  const lavs = ["A", "D", "E", "M"].filter((l) => has(`cabLav${l}Locked`) && (l !== "M" || on("cabLavMFlush")));
  const preselect = has("cabWaterPreselect") ? Math.round(v.cabWaterPreselect / 25) * 25 : undefined;
  const waste = on("cabWasteFull")
    ? t("techlog.cabin.waste_full")
    : (v.cabWastePct ?? 0) > 3
      ? t("techlog.cabin.waste_service")
      : t("techlog.cabin.waste_empty");
  const num = (x: number | undefined, d = 0) =>
    x === undefined ? "—" : x.toLocaleString(undefined, { maximumFractionDigits: d, minimumFractionDigits: d });
  const tone = (s: string) =>
    s === "open"
      ? "border-rose-500/40 bg-rose-500/15 text-rose-300"
      : s === "disarmed"
        ? "border-amber-500/40 bg-amber-500/15 text-amber-300"
        : "border-emerald-500/30 bg-emerald-500/10 text-emerald-300";

  const Pill = ({ label, ok, warn }: { label: string; ok: boolean; warn?: boolean }) => (
    <span
      className={cn(
        "rounded-md border px-1.5 py-0.5 text-[10px]",
        warn
          ? "border-rose-500/40 bg-rose-500/15 text-rose-300"
          : ok
            ? "border-brand-500/40 bg-brand-500/10 text-brand-200"
            : "border-sf-border bg-sf-raised text-sf-muted",
      )}
    >
      {label}
    </span>
  );
  const Choice = ({
    k,
    label,
    current,
    disabled,
    onPick,
  }: {
    k: string;
    label: string;
    current: boolean;
    disabled: boolean;
    onPick: () => void;
  }) => (
    <button
      disabled={disabled || busy !== null}
      onClick={onPick}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40",
        current
          ? "border-brand-500/50 bg-brand-500/15 text-brand-200"
          : "border-sf-border bg-sf-raised text-sf-text hover:border-brand-500/40",
      )}
    >
      {busy === k && <Loader2 className="h-3 w-3 animate-spin" />}
      {label}
    </button>
  );
  const Tank = ({ label, pct, kg, mark, cls }: { label: string; pct?: number; kg?: number; mark?: number; cls: string }) => (
    <div className="min-w-0 flex-1">
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="text-sf-text">{label}</span>
        <span className="tabular-nums text-sf-muted">
          {pct === undefined ? "—" : `${num(pct)} %`} · {num(kg)} kg
        </span>
      </div>
      <div className="relative mt-1 h-1.5 overflow-hidden rounded-full bg-sf-raised">
        <div className={cn("h-full rounded-full", cls)} style={{ width: `${Math.max(2, Math.min(100, pct ?? 0))}%` }} />
        {mark !== undefined && (
          <div className="absolute inset-y-0 w-0.5 bg-sf-text/80" style={{ left: `${Math.min(99, mark)}%` }} />
        )}
      </div>
    </div>
  );

  return (
    <Section
      icon={<Armchair className="h-4 w-4 text-brand-300" />}
      title={t("techlog.cabin.title")}
      subtitle={t("techlog.cabin.subtitle")}
    >
      {cab === undefined ? (
        <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.cabin.loading")} />
      ) : !cab.live ? (
        <Empty icon={<Clock className="h-4 w-4" />} text={t("techlog.cabin.waiting")} />
      ) : (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          {/* Agua y residuos. */}
          <div className="space-y-2">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("techlog.cabin.water_waste")}</div>
            <div className="flex gap-3">
              <Tank label={t("techlog.cabin.water")} pct={v.cabWaterPct} kg={v.cabWaterKg} mark={preselect} cls="bg-sky-400" />
              <Tank label={t("techlog.cabin.waste")} pct={v.cabWastePct} kg={v.cabWasteKg} cls={on("cabWasteFull") ? "bg-rose-400" : "bg-amber-400"} />
            </div>
            <div className={cn("text-[11px]", on("cabWasteFull") ? "text-rose-300" : "text-sf-muted")}>
              {t("techlog.cabin.waste")}: {waste}
            </div>
            <div className="text-[11px] text-sf-muted">{t("techlog.cabin.preselect")}</div>
            <div className="flex flex-wrap gap-1">
              {WATER_STEPS.map((n) => (
                <Choice
                  key={n}
                  k={`preselect-${n}`}
                  label={`${n} %`}
                  current={preselect === n}
                  disabled={!cab.canSetPreselect}
                  onPick={() => void set("preselect", n)}
                />
              ))}
            </div>
            <p className="text-[10px] text-sf-muted">{t("techlog.cabin.preselect_hint")}</p>
            <div className="flex flex-wrap gap-1">
              <Pill label={t(on("cabVacuumPowered") ? "techlog.cabin.vacuum_on" : "techlog.cabin.vacuum_off")} ok={on("cabVacuumPowered")} />
              {has("cabVacuumGen") && !on("cabVacuumGen") && <Pill label={t("techlog.cabin.vacgen_failed")} ok={false} warn />}
              {on("cabWaterPanel") && <Pill label={t("techlog.cabin.water_panel_open")} ok />}
              {on("cabWastePanel") && <Pill label={t("techlog.cabin.waste_panel_open")} ok />}
            </div>
          </div>

          {/* Puertas, toboganes y baños. */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("techlog.cabin.doors")}</span>
              <span className={cn("rounded-md border px-1.5 py-0.5 text-[10px]", tone(overall))}>
                {t(`techlog.cabin.doors_${overall}`)}
              </span>
            </div>
            <div className="flex flex-wrap gap-1">
              {doors.map((d) => (
                <span
                  key={d}
                  title={t(`techlog.cabin.door_${doorState(d)}`)}
                  className={cn("rounded-md border px-1.5 py-0.5 font-mono text-[10px]", tone(doorState(d)))}
                >
                  {d}
                </span>
              ))}
              {(["L", "R"] as const)
                .filter((s) => has(`cabSlideOW${s}`))
                .map((s) => (
                  <span
                    key={s}
                    title={t(on(`cabSlideOW${s}`) ? "techlog.cabin.door_ok" : "techlog.cabin.door_disarmed")}
                    className={cn("rounded-md border px-1.5 py-0.5 font-mono text-[10px]", tone(on(`cabSlideOW${s}`) ? "ok" : "disarmed"))}
                  >
                    OW {s}
                  </span>
                ))}
            </div>
            {lavs.length > 0 && (
              <>
                <div className="pt-1 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("techlog.cabin.lavs")}</div>
                <ul className="space-y-1">
                  {lavs.map((l) => (
                    <li key={l} className="flex items-center gap-1.5 text-[11px]">
                      <span className="w-4 font-mono text-sf-text">{l}</span>
                      <Pill
                        label={t(on(`cabLav${l}Occupied`) ? "techlog.cabin.lav_occupied" : on(`cabLav${l}Locked`) ? "techlog.cabin.lav_locked" : "techlog.cabin.lav_free")}
                        ok={on(`cabLav${l}Occupied`)}
                      />
                      {!on(`cabLav${l}Flush`) && <Pill label={t("techlog.cabin.flush_failed")} ok={false} warn />}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>

          {/* Temperatura y presión. */}
          <div className="space-y-1.5">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("techlog.cabin.temp")}</div>
            {[
              { k: "fwd", air: v.cabTempFwdAir, sel: v.cabTempFwdSel, seat: v.cabTempFwdSeat },
              { k: "aft", air: v.cabTempAftAir, sel: v.cabTempAftSel, seat: undefined },
            ].map((z) => (
              <div key={z.k} className="flex flex-wrap items-baseline gap-x-2 text-xs">
                <span className="w-20 text-sf-text">{t(`techlog.cabin.${z.k}`)}</span>
                <span className="font-semibold tabular-nums text-sf-text">{num(z.air, 1)} °C</span>
                {z.sel !== undefined && (
                  <span className="text-[11px] text-sf-muted">{t("techlog.cabin.selected", { v: `${num(z.sel, 1)} °C` })}</span>
                )}
                {z.seat !== undefined && (
                  <span className="text-[11px] text-sf-muted">{t("techlog.cabin.seats", { v: `${num(z.seat, 1)} °C` })}</span>
                )}
              </div>
            ))}
            {has("cabPressure") && (
              <div className="text-[11px] text-sf-muted">
                {t("techlog.cabin.pressure", { p: num(v.cabPressure), d: num(v.cabDiffPressure, 2) })}
              </div>
            )}
            <p className="text-[10px] text-sf-muted">{t("techlog.cabin.temp_hint")}</p>
          </div>

          {/* Luces y señales. */}
          <div className="space-y-1.5">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("techlog.cabin.lights")}</div>
            <div className="flex flex-wrap gap-1">
              {CABIN_LIGHTS.map((l) => (
                <Choice
                  key={l}
                  k={`lights-${l}`}
                  label={t(`techlog.cabin.light_${l}`)}
                  current={cab.lights === l}
                  disabled={!cab.canSetLights}
                  onPick={() => void set("lights", l)}
                />
              ))}
            </div>
            <div className="flex flex-wrap gap-1 pt-1">
              <Pill label={t(on("cabSeatbelts") ? "techlog.cabin.seatbelts_on" : "techlog.cabin.seatbelts_off")} ok={on("cabSeatbelts")} />
              <Pill label={t(on("cabNoSmoking") ? "techlog.cabin.nosmoking_on" : "techlog.cabin.nosmoking_off")} ok={on("cabNoSmoking")} />
              <Pill label={t(on("cabReady") ? "techlog.cabin.ready" : "techlog.cabin.not_ready")} ok={on("cabReady")} />
              {on("cabCalled") && <Pill label={t("techlog.cabin.called")} ok />}
              {has("cabCids") && !on("cabCids") && <Pill label={t("techlog.cabin.cids_failed")} ok={false} warn />}
              {has("cabFap") && !on("cabFap") && <Pill label={t("techlog.cabin.fap_failed")} ok={false} warn />}
            </div>
            <p className="text-[10px] text-sf-muted">{t("techlog.cabin.signs_hint")}</p>
          </div>
        </div>
      )}
    </Section>
  );
}

function Section({
  icon,
  title,
  subtitle,
  action,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <header className="mb-2 flex items-center gap-2">
        {icon}
        <h3 className="text-sm font-semibold text-sf-text">{title}</h3>
        {subtitle && <span className="text-[11px] text-sf-muted">{subtitle}</span>}
        <div className="ml-auto">{action}</div>
      </header>
      {children}
    </section>
  );
}

function Empty({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <div className="flex items-center gap-2 py-2 text-xs text-sf-muted">
      {icon}
      {text}
    </div>
  );
}

/** (19/10) FlightBook: veredicto de TechLog del vuelo (nota, eventos, desgaste). */
export function TechLogFlightCard({
  flight,
  own = false,
  fallback,
}: {
  flight: FlightLogEntry;
  /** (19/10) true = Tech Log de SimFleet (registro por id de vuelo). */
  own?: boolean;
  /** Qué mostrar si el vuelo no tiene registro (vuelos anteriores al sistema). */
  fallback?: React.ReactNode;
}) {
  const epoch = useTechLogStore((s) => s.epoch);
  const [data, setData] = useState<TechLogFlight | null | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    const load = own
      ? api.mxFlight(flight.id)
      : flight.aircraftRegistration
        ? api.techlogFlight(flight.aircraftRegistration, flight.startedAt, flight.endedAt)
        : Promise.resolve(null);
    load.then((r) => alive && setData(r)).catch(() => alive && setData(null));
    return () => {
      alive = false;
    };
  }, [own, flight.id, flight.aircraftRegistration, flight.startedAt, flight.endedAt, epoch]);
  if (data === undefined) return null;
  // (21/10) Con `fallback` (aunque sea null) se muestra eso en lugar del aviso.
  if (data === null && (own || fallback !== undefined)) return <>{fallback ?? null}</>;
  if (data === null) {
    return (
      <div className="rounded-lg border border-sf-border bg-sf-surface px-3 py-2 text-[11px] text-sf-muted">
        {t("techlog.flight.none")}
      </div>
    );
  }
  const warnings = data.events.filter((e) => e.severity !== "Info");
  const score = data.score != null ? Math.round(data.score) : null;
  return (
    <div className="rounded-lg border border-sf-border bg-sf-surface px-3 py-2.5">
      <div className="flex items-center gap-2">
        <Wrench className="h-3.5 w-3.5 text-brand-300" />
        <span className="text-xs font-semibold text-sf-text">{data.appName}</span>
        {score != null && (
          <span
            className={cn(
              "rounded px-1.5 py-0.5 font-mono text-[11px] font-bold",
              score >= 85 ? "bg-emerald-500/20 text-emerald-300" : score >= 65 ? "bg-amber-500/20 text-amber-300" : "bg-rose-500/20 text-rose-300",
            )}
          >
            {score}/100
          </span>
        )}
        {warnings.length > 0 && (
          <span className="text-[11px] text-amber-300">{t("techlog.flight.warnings", { n: warnings.length })}</span>
        )}
        {data.dispatchViolation && (
          <span className="inline-flex items-center gap-1 rounded bg-rose-500/20 px-1.5 py-0.5 text-[10px] font-bold uppercase text-rose-300">
            <ShieldAlert className="h-3 w-3" />
            {t("techlog.flight.violation")}
          </span>
        )}
      </div>
      {data.summary && <p className="mt-1 text-[11px] text-sf-muted">{data.summary}</p>}
      {warnings.length > 0 && (
        <ul className="mt-1.5 space-y-0.5 text-[11px]">
          {warnings.slice(0, 6).map((e, i) => (
            <li key={i} className={e.severity === "Warning" ? "text-rose-300" : "text-amber-300"}>
              • {e.phase ? `${e.phase}: ` : ""}
              {e.text}
            </li>
          ))}
        </ul>
      )}
      {data.wear.length > 0 && (
        <p className="mt-1.5 text-[10px] text-sf-muted">
          {t("techlog.flight.wear")}{" "}
          {data.wear
            .slice(0, 5)
            .map((w) => `${w.item} ${Math.abs(Math.round(w.amount * 10) / 10)} ${w.unit}`)
            .join(" · ")}
        </p>
      )}
    </div>
  );
}
