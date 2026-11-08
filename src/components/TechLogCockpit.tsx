import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BookmarkPlus, ChevronLeft, ChevronRight, Loader2, Pencil, RefreshCw, RotateCcw, Save, SlidersHorizontal, Trash2 } from "lucide-react";
import type {
  TechLogAircraft,
  TechLogCockpit as Cockpit,
  TechLogCockpitControl,
  TechLogCockpitPage,
  TechLogCockpitPanel,
  TechLogCockpitRequest,
  TechLogCockpitZone,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useTechLogStore } from "../stores/useTechLogStore";
import { useToastStore } from "../stores/useToastStore";
import { actionError, Empty, fmtDate, Section } from "./TechLogPanel";
import { TechLogPageNotice } from "./TechLogTabs";
import { DetailModal } from "./DetailModal";

/**
 * (01/11) Etapa 5a — página «Cockpit» de Fenix/PMDG TechLog en SimFleet:
 * perfiles del estado del cockpit (guardar, leer en vivo, restaurar, al
 * cargar en frío y oscuro) y cada mando guardado frente al real. Sólo con el
 * avión ACTIVO de TechLog; leer y mover piden el avión cargado en el sim.
 * El esquema del cockpit es un dibujo propio de SimFleet.
 */

type CtlState = "same" | "differs" | "none" | "skip";

function ctlState(c: TechLogCockpitControl): CtlState {
  if (!c.keep) return "skip";
  if (c.never || c.saved == null || c.live == null) return "none";
  if (c.kind === "level") return Math.abs(c.saved - c.live) <= (c.max || 1) * 0.03 ? "same" : "differs";
  return Math.abs(c.saved - c.live) < 0.001 ? "same" : "differs";
}

function valueLabel(c: TechLogCockpitControl, v: number | null): string {
  if (v == null) return "—";
  if (c.kind === "level") return `${Math.round((v / (c.max || 1)) * 100)}%`;
  const p = c.positions.find((x) => Math.abs(x.value - v) < 0.001);
  return p?.label || String(Math.round(v * 100) / 100);
}

/** Siguiente / anterior posición de un mando (o un 5 % si es continuo). */
function stepValue(c: TechLogCockpitControl, dir: 1 | -1): number | null {
  if (c.live == null) return null;
  if (c.kind === "level") {
    const max = c.max || 1;
    const v = Math.round((c.live + (dir * max) / 20) * 100) / 100;
    return v < 0 || v > max ? null : v;
  }
  const values = [...new Set(c.positions.map((p) => p.value))].sort((a, b) => a - b);
  if (values.length === 0) return null;
  const next = dir > 0 ? values.find((v) => v > c.live! + 0.001) : [...values].reverse().find((v) => v < c.live! - 0.001);
  return next ?? null;
}

function counts(controls: TechLogCockpitControl[]) {
  const n = { same: 0, differs: 0, none: 0, skip: 0 };
  for (const c of controls) n[ctlState(c)] += 1;
  return n;
}

const DOT: Record<CtlState, string> = {
  same: "bg-emerald-400",
  differs: "bg-amber-400",
  none: "bg-slate-500",
  skip: "bg-transparent ring-1 ring-slate-500",
};

/** (02/11) Cockpit propio: nombres de sus paneles e interruptores estándar. */
function stdName(kind: "panel" | "ctl", key: string, fallback: string): string {
  const k = kind === "panel" ? `techlog.cockpit.std_panel.${key}` : `techlog.cockpit.std.${key}`;
  const v = t(k);
  return v === k ? fallback : v;
}

/** (02/11) Cockpit propio: lo que responde el backend → texto. */
function stdMessage(msg: string): string | undefined {
  if (!msg) return undefined;
  if (msg === "saved") return t("techlog.cockpit.std_saved");
  const moved = /^moved:(\d+)$/.exec(msg);
  if (moved) return t("techlog.cockpit.std_moved", { n: moved[1] });
  if (msg.includes("not_in_sim")) return t("techlog.cockpit.not_in_sim");
  if (msg.includes("no_profile")) return t("techlog.cockpit.std_no_profile");
  return msg;
}

function zoneName(z: TechLogCockpitZone): string {
  const k = `techlog.cockpit.zone_${z.key}`;
  const v = t(k);
  return v === k ? z.name : v;
}

export function TechLogCockpit({ data, refreshKey }: { data: TechLogAircraft; refreshKey: number }) {
  const pushToast = useToastStore((s) => s.push);
  const epoch = useTechLogStore((s) => s.epoch);
  const [page, setPage] = useState<TechLogCockpitPage | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [openPanel, setOpenPanel] = useState<string | null>(null);
  // (03/11) Zona abierta en el modal.
  const [zoneFocus, setZoneFocus] = useState<string | null>(null);
  // (02/11) Tech Log propio: interruptores estándar del simulador.
  const own = data.source === "simfleet";

  const load = useCallback(async () => {
    try {
      setPage(await api.techlogCockpit(data.registration));
    } catch {
      setPage({ state: "unsupported", source: data.source, activeRegistration: null, cockpit: null });
    }
  }, [data.registration, data.source]);

  useEffect(() => {
    void load();
  }, [load, epoch, refreshKey, data.running, data.active]);

  // Refresco: rápido mientras TechLog trabaja, lento si no.
  const cp = page?.cockpit ?? null;
  const live = page?.state === "live";
  useEffect(() => {
    const ms = live ? (cp?.busy != null ? 1500 : 8000) : 15000;
    const id = window.setInterval(() => void load(), ms);
    return () => window.clearInterval(id);
  }, [load, live, cp?.busy]);

  const run = useCallback(
    async (key: string, req: Omit<TechLogCockpitRequest, "registration">, silent = false) => {
      setBusyKey(key);
      try {
        const r = await api.techlogCockpitAction({ registration: data.registration, ...req });
        if (!r.ok || !silent) {
          pushToast({
            kind: r.ok ? "success" : "error",
            title: r.ok ? t("techlog.cockpit.done") : t("techlog.cockpit.refused"),
            message: own ? stdMessage(r.message) : r.message || undefined,
          });
        }
      } catch (e) {
        pushToast({ kind: "error", title: t("techlog.cockpit.refused"), message: own ? stdMessage(String(e)) : actionError(String(e)) });
      } finally {
        setBusyKey(null);
        void load();
      }
    },
    [data.registration, load, pushToast, own],
  );

  // Como TechLog: al abrir la página con el avión en el sim, lee el cockpit.
  const didRead = useRef(false);
  useEffect(() => {
    if (!didRead.current && live && cp?.connected && cp.busy == null) {
      didRead.current = true;
      void run("read", { action: "read" }, true);
    }
  }, [live, cp?.connected, cp?.busy, run]);

  if (page === null) {
    return <Empty icon={<Loader2 className="h-4 w-4 animate-spin" />} text={t("techlog.cockpit.loading")} />;
  }
  if (!live || !cp) {
    return (
      <TechLogPageNotice
        state={page.state}
        appName={data.appName}
        activeRegistration={page.activeRegistration}
        what="cockpit"
      />
    );
  }

  const panel = openPanel
    ? cp.zones.flatMap((z) => z.panels).find((p) => p.key === openPanel) ?? null
    : null;
  const zone = cp.zones.find((z) => z.key === zoneFocus) ?? null;

  return (
    <div className="space-y-3">
      <ProfileBar
        cp={cp}
        own={own}
        otherLoaded={page.activeRegistration}
        busyKey={busyKey}
        name={name}
        setName={setName}
        confirmDelete={confirmDelete}
        setConfirmDelete={setConfirmDelete}
        run={run}
      />
      {/* (03/11) El dibujo aquí; cada zona, con sus paneles, en un modal. */}
      <Section
        icon={<SlidersHorizontal className="h-4 w-4 text-brand-300" />}
        title={t("techlog.cockpit.title")}
        subtitle={t("techlog.cockpit.subtitle", { n: cp.zones.reduce((a, z) => a + z.panels.reduce((b, p) => b + p.controls.length, 0), 0) })}
      >
        <CockpitSchematic
          zones={cp.zones}
          focus={zoneFocus}
          onPick={(key) => {
            setZoneFocus(key);
            setOpenPanel(null);
          }}
        />
      </Section>
      {zone && (
        <DetailModal
          title={t("techlog.cockpit.title")}
          subtitle={cp.registration ?? undefined}
          icon={<SlidersHorizontal className="h-4 w-4" />}
          tabs={cp.zones.map((z) => {
            const differs = counts(z.panels.flatMap((p) => p.controls)).differs;
            return { id: z.key, label: zoneName(z), badge: differs > 0 ? differs : undefined };
          })}
          tab={zone.key}
          onTab={(id) => {
            setZoneFocus(id);
            setOpenPanel(null);
          }}
          onClose={() => {
            setZoneFocus(null);
            setOpenPanel(null);
          }}
        >
          {panel ? (
            <PanelDetail panel={panel} cp={cp} own={own} busyKey={busyKey} run={run} onBack={() => setOpenPanel(null)} />
          ) : (
            <>
              <p className="mb-3 text-[11px] text-sf-muted">
                {t("techlog.cockpit.zone_counts", {
                  n: zone.panels.flatMap((p) => p.controls).length,
                  d: counts(zone.panels.flatMap((p) => p.controls)).differs,
                })}
              </p>
              <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3">
                {zone.panels.map((p) => (
                  <PanelCard key={p.key} panel={p} own={own} onOpen={() => setOpenPanel(p.key)} />
                ))}
              </div>
            </>
          )}
        </DetailModal>
      )}
    </div>
  );
}

type RunFn = (key: string, req: Omit<TechLogCockpitRequest, "registration">, silent?: boolean) => Promise<void>;

function ProfileBar({
  cp,
  own,
  otherLoaded,
  busyKey,
  name,
  setName,
  confirmDelete,
  setConfirmDelete,
  run,
}: {
  cp: Cockpit;
  own: boolean;
  /** Matrícula de otro avión cargado en el simulador (Tech Log propio). */
  otherLoaded: string | null;
  busyKey: string | null;
  name: string;
  setName: (v: string) => void;
  confirmDelete: boolean;
  setConfirmDelete: (v: boolean) => void;
  run: RunFn;
}) {
  const idle = cp.connected && cp.busy == null && busyKey === null;
  const p = cp.profiles.find((x) => x.id === cp.profileId) ?? null;
  const typed = name.trim();
  const btn =
    "inline-flex items-center gap-1 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40";
  const plain = cn(btn, "border-sf-border bg-sf-raised text-sf-text hover:border-brand-500/40");
  const spin = (k: string) => busyKey === k && <Loader2 className="h-3 w-3 animate-spin" />;
  return (
    <section className="space-y-2 rounded-xl border border-sf-border bg-sf-bg/40 p-3">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-sf-text">{cp.registration ?? "—"}</div>
          <div className="text-[11px] text-sf-muted">
            {p ? `${p.name} · ` : ""}
            {cp.savedUtc
              ? cp.savedAirport
                ? t("techlog.cockpit.saved_at", { d: fmtWhen(cp.savedUtc), apt: cp.savedAirport })
                : t("techlog.cockpit.saved", { d: fmtWhen(cp.savedUtc) })
              : t("techlog.cockpit.nothing_saved")}
            {cp.liveUtc && ` · ${t("techlog.cockpit.read_at", { d: fmtWhen(cp.liveUtc) })}`}
          </div>
        </div>
        <label className="inline-flex cursor-pointer items-center gap-1.5 text-[11px] text-sf-text">
          <input
            type="checkbox"
            className="accent-brand-500"
            checked={cp.restore}
            disabled={busyKey !== null}
            onChange={(e) => void run("auto", { action: "auto", on: e.target.checked })}
          />
          {t("techlog.cockpit.restore_cold_dark")}
        </label>
      </div>

      {/* Perfiles */}
      <div className="flex flex-wrap items-center gap-1.5">
        {cp.profiles.map((x) => (
          <button
            key={x.id}
            disabled={busyKey !== null}
            onClick={() => {
              setConfirmDelete(false);
              void run(`select-${x.id}`, { action: "select", profile: x.id }, true);
            }}
            className={cn(
              btn,
              x.id === cp.profileId
                ? "border-brand-500/60 bg-brand-500/15 text-brand-200"
                : "border-sf-border bg-sf-raised text-sf-text hover:border-brand-500/40",
            )}
          >
            {spin(`select-${x.id}`)}
            {x.name}
            {x.auto && <span className="text-[10px] text-sf-muted">· {t("techlog.cockpit.on_load")}</span>}
          </button>
        ))}
        {cp.profiles.length === 0 && <span className="text-[11px] text-sf-muted">{t("techlog.cockpit.no_profiles")}</span>}
      </div>
      {p && (
        <div className="flex flex-wrap items-center gap-2">
          <label className="inline-flex cursor-pointer items-center gap-1.5 text-[11px] text-sf-text">
            <input
              type="checkbox"
              className="accent-brand-500"
              checked={p.auto}
              disabled={busyKey !== null}
              onChange={() => void run(`pauto-${p.id}`, { action: "profileAuto", profile: p.id }, true)}
            />
            {t("techlog.cockpit.profile_on_load", { name: p.name })}
          </label>
          <button
            className={plain}
            disabled={!typed || busyKey !== null}
            title={typed ? undefined : t("techlog.cockpit.rename_hint")}
            onClick={() => {
              void run(`rename-${p.id}`, { action: "rename", profile: p.id, name: typed });
              setName("");
            }}
          >
            {spin(`rename-${p.id}`) || <Pencil className="h-3 w-3" />}
            {t("techlog.cockpit.rename")}
          </button>
          {confirmDelete ? (
            <>
              <button
                className={cn(btn, "border-rose-500/50 bg-rose-500/15 text-rose-200 hover:bg-rose-500/25")}
                disabled={busyKey !== null}
                onClick={() => {
                  setConfirmDelete(false);
                  void run(`delete-${p.id}`, { action: "delete", profile: p.id });
                }}
              >
                <Trash2 className="h-3 w-3" />
                {t("techlog.cockpit.delete_confirm", { name: p.name })}
              </button>
              <button className={plain} onClick={() => setConfirmDelete(false)}>
                {t("common.cancel")}
              </button>
            </>
          ) : (
            <button className={plain} disabled={busyKey !== null} onClick={() => setConfirmDelete(true)}>
              <Trash2 className="h-3 w-3" />
              {t("techlog.cockpit.delete")}
            </button>
          )}
        </div>
      )}

      {/* Guardar / leer / restaurar */}
      <div className="flex flex-wrap items-center gap-1.5">
        <input
          value={name}
          maxLength={40}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("techlog.cockpit.new_name")}
          className="w-52 rounded-md border border-sf-border bg-sf-bg px-2 py-1 text-[11px] text-sf-text placeholder:text-sf-muted focus:border-brand-500/60 focus:outline-none"
        />
        <button
          className={cn(btn, "border-brand-500/50 bg-brand-500/15 text-brand-200 hover:bg-brand-500/25")}
          disabled={!idle || !typed}
          onClick={() => {
            void run("save-new", { action: "save", name: typed });
            setName("");
          }}
        >
          {spin("save-new") || <BookmarkPlus className="h-3 w-3" />}
          {t("techlog.cockpit.save_new")}
        </button>
        {p && (
          <button className={plain} disabled={!idle} onClick={() => void run("save-p", { action: "save", profile: p.id })}>
            {spin("save-p") || <Save className="h-3 w-3" />}
            {t("techlog.cockpit.update", { name: p.name })}
          </button>
        )}
        <button className={plain} disabled={!idle} onClick={() => void run("read", { action: "read" })}>
          {spin("read") || <RefreshCw className="h-3 w-3" />}
          {t("techlog.cockpit.read")}
        </button>
        <button
          className={plain}
          disabled={!idle || !cp.savedUtc}
          title={t("techlog.cockpit.restore_hint")}
          onClick={() => void run("restore", { action: "restore", profile: cp.profileId })}
        >
          {spin("restore") || <RotateCcw className="h-3 w-3" />}
          {p ? t("techlog.cockpit.restore_profile", { name: p.name }) : t("techlog.cockpit.restore_now")}
        </button>
        <span className="ml-auto inline-flex items-center gap-2.5 text-[10px] text-sf-muted">
          {(["same", "differs", "none"] as const).map((k) => (
            <span key={k} className="inline-flex items-center gap-1">
              <span className={cn("h-2 w-2 rounded-full", DOT[k])} />
              {t(`techlog.cockpit.legend_${k}`)}
            </span>
          ))}
        </span>
      </div>

      {!cp.connected && <p className="text-[11px] text-amber-300">{t("techlog.cockpit.not_in_sim")}</p>}
      {own && !cp.connected && otherLoaded && (
        <p className="text-[11px] text-amber-300">{t("techlog.cockpit.std_other_loaded", { reg: otherLoaded })}</p>
      )}
      {own && <p className="text-[10px] text-sf-muted">{t("techlog.cockpit.std_hint")}</p>}
      {cp.busy != null && (
        <p className="inline-flex items-center gap-1.5 text-[11px] text-brand-200">
          <Loader2 className="h-3 w-3 animate-spin" />
          {t("techlog.cockpit.busy", { what: cp.busy || "…" })}
        </p>
      )}
      {cp.busy == null && cp.last && <p className="text-[11px] text-sf-muted">{cp.last}</p>}
    </section>
  );
}

function fmtWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return fmtDate(iso);
  return d.toLocaleString(undefined, { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

function PanelCard({ panel, own, onOpen }: { panel: TechLogCockpitPanel; own: boolean; onOpen: () => void }) {
  const n = counts(panel.controls);
  const worst: CtlState = n.differs > 0 ? "differs" : n.same > 0 && n.none === 0 ? "same" : "none";
  return (
    <button
      onClick={onOpen}
      className="group flex flex-col gap-1.5 rounded-lg border border-sf-border bg-sf-surface p-2.5 text-left transition-colors hover:border-brand-500/50"
    >
      <div className="flex items-center gap-1.5">
        <span className={cn("h-2 w-2 shrink-0 rounded-full", DOT[worst])} />
        <span className="min-w-0 flex-1 truncate text-xs font-semibold text-sf-text">
          {own ? stdName("panel", panel.key, panel.name) : panel.name}
        </span>
        {n.differs > 0 && <span className="text-[10px] font-medium text-amber-300">{t("techlog.cockpit.n_differ", { n: n.differs })}</span>}
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sf-muted group-hover:text-brand-300" />
      </div>
      <div className="flex flex-wrap gap-1">
        {panel.controls.map((c) => {
          const st = ctlState(c);
          return (
            <span
              key={c.id}
              title={c.id}
              className={cn(
                "rounded px-1.5 py-0.5 font-mono text-[9.5px] leading-tight",
                st === "differs"
                  ? "bg-amber-500/15 text-amber-200"
                  : st === "same"
                    ? "bg-emerald-500/10 text-emerald-200/90"
                    : st === "skip"
                      ? "bg-sf-raised/40 text-sf-muted/60 line-through"
                      : "bg-sf-raised/60 text-sf-muted",
              )}
            >
              {own ? stdName("ctl", c.id, c.name) : c.name} {valueLabel(c, c.live ?? c.saved)}
            </span>
          );
        })}
      </div>
    </button>
  );
}

function PanelDetail({
  panel,
  cp,
  own,
  busyKey,
  run,
  onBack,
}: {
  panel: TechLogCockpitPanel;
  cp: Cockpit;
  own: boolean;
  busyKey: string | null;
  run: RunFn;
  onBack: () => void;
}) {
  const idle = cp.connected && cp.busy == null && busyKey === null;
  const mini =
    "rounded border border-sf-border bg-sf-raised px-1.5 py-0.5 text-[10px] text-sf-text hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-35";
  return (
    <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <header className="mb-3 flex flex-wrap items-center gap-2">
        <button onClick={onBack} className="inline-flex items-center gap-0.5 rounded-md px-1.5 py-1 text-[11px] text-sf-muted hover:bg-sf-raised hover:text-sf-text">
          <ChevronLeft className="h-3.5 w-3.5" />
          {t("techlog.cockpit.back")}
        </button>
        <h3 className="text-sm font-semibold text-sf-text">{own ? stdName("panel", panel.key, panel.name) : panel.name}</h3>
        <button
          className="ml-auto inline-flex items-center gap-1 rounded-md border border-sf-border bg-sf-raised px-2 py-1 text-[11px] text-sf-text hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={!idle || !cp.savedUtc}
          onClick={() => void run(`restore-${panel.key}`, { action: "restore", id: panel.key })}
        >
          {busyKey === `restore-${panel.key}` ? <Loader2 className="h-3 w-3 animate-spin" /> : <RotateCcw className="h-3 w-3" />}
          {t("techlog.cockpit.panel_to_saved")}
        </button>
      </header>
      <div className="overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="text-left text-[10px] uppercase tracking-wide text-sf-muted">
              <th className="py-1.5 pr-2 font-medium">{t("techlog.cockpit.col_switch")}</th>
              <th className="py-1.5 pr-2 font-medium">{t("techlog.cockpit.col_saved")}</th>
              <th className="py-1.5 pr-2 font-medium">{t("techlog.cockpit.col_live")}</th>
              <th className="py-1.5 pr-2 font-medium">{t("techlog.cockpit.col_set")}</th>
              <th className="py-1.5 font-medium">{t("techlog.cockpit.col_keep")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-sf-border/50">
            {panel.controls.map((c) => {
              const st = ctlState(c);
              const down = stepValue(c, -1);
              const up = stepValue(c, 1);
              return (
                <tr key={c.id} className={st === "differs" ? "bg-amber-500/5" : undefined}>
                  <td className="py-1.5 pr-2">
                    <div className="flex items-center gap-1.5">
                      <span className={cn("h-2 w-2 shrink-0 rounded-full", DOT[st])} />
                      <span className="text-sf-text">{own ? stdName("ctl", c.id, c.name) : c.name}</span>
                    </div>
                    <div className="pl-3.5 font-mono text-[9.5px] text-sf-muted">{c.id}</div>
                  </td>
                  <td className="py-1.5 pr-2 tabular-nums text-sf-muted">{valueLabel(c, c.saved)}</td>
                  <td className={cn("py-1.5 pr-2 tabular-nums", st === "differs" ? "font-semibold text-amber-300" : "text-sf-text")}>
                    {valueLabel(c, c.live)}
                  </td>
                  <td className="py-1.5 pr-2">
                    <span className="inline-flex gap-1">
                      <button className={mini} disabled={!idle || down == null} onClick={() => down != null && void run(`set-${c.id}`, { action: "set", id: c.id, value: down }, true)}>
                        −
                      </button>
                      <button className={mini} disabled={!idle || up == null} onClick={() => up != null && void run(`set-${c.id}`, { action: "set", id: c.id, value: up }, true)}>
                        +
                      </button>
                      {c.saved != null && (
                        <button
                          className={mini}
                          disabled={!idle || st === "same"}
                          onClick={() => void run(`set-${c.id}`, { action: "set", id: c.id, value: c.saved! }, true)}
                        >
                          {t("techlog.cockpit.as_saved")}
                        </button>
                      )}
                      {busyKey === `set-${c.id}` && <Loader2 className="h-3 w-3 animate-spin text-sf-muted" />}
                    </span>
                  </td>
                  <td className="py-1.5">
                    <input
                      type="checkbox"
                      className="accent-brand-500"
                      checked={c.keep}
                      disabled={busyKey !== null}
                      onChange={(e) => void run(`keep-${c.id}`, { action: "keep", id: c.id, keep: e.target.checked }, true)}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {!cp.connected && <p className="mt-2 text-[11px] text-amber-300">{t("techlog.cockpit.not_in_sim")}</p>}
    </section>
  );
}

/**
 * Dibujo propio del cockpit visto desde arriba: panel superior, visera,
 * panel principal y pedestal, cada zona coloreada por sus mandos (verde =
 * todo igual que lo guardado, ámbar = hay distintos, gris = sin leer).
 */
/** Trazos del dibujo: el borde del tema con más presencia (claro y oscuro). */
const LINE = "rgb(var(--sf-muted) / 0.45)";

function CockpitSchematic({
  zones,
  focus,
  onPick,
}: {
  zones: TechLogCockpitZone[];
  focus: string | null;
  onPick: (key: string) => void;
}) {
  const byKey = useMemo(() => new Map(zones.map((z) => [z.key, z])), [zones]);
  const tone = (key: string) => {
    const z = byKey.get(key);
    if (!z) return { fill: "rgb(var(--sf-raised) / 0.35)", stroke: "rgb(var(--sf-border) / 0.25)", label: "rgb(var(--sf-muted))", n: null };
    const n = counts(z.panels.flatMap((p) => p.controls));
    if (n.differs > 0) return { fill: "rgba(251,191,36,0.16)", stroke: "rgba(251,191,36,0.7)", label: "#fcd34d", n };
    if (n.same > 0 && n.none === 0) return { fill: "rgba(52,211,153,0.13)", stroke: "rgba(52,211,153,0.6)", label: "#6ee7b7", n };
    return { fill: "rgb(var(--sf-raised) / 0.6)", stroke: "rgb(var(--sf-muted) / 0.5)", label: "rgb(var(--sf-text))", n };
  };
  const Zone = ({ k, x, y, w, h, r = 8, ly, inline }: { k: string; x: number; y: number; w: number; h: number; r?: number; ly?: number; inline?: boolean }) => {
    const c = tone(k);
    const z = byKey.get(k);
    return (
      <g
        role={z ? "button" : undefined}
        tabIndex={z ? 0 : undefined}
        onClick={() => z && onPick(k)}
        onKeyDown={(e) => z && (e.key === "Enter" || e.key === " ") && onPick(k)}
        className={z ? "cursor-pointer" : undefined}
      >
        <rect x={x} y={y} width={w} height={h} rx={r} style={{ fill: c.fill, stroke: c.stroke }} strokeWidth={focus === k ? 2.2 : 1.2} />
        <text x={x + w / 2} y={ly ?? y + h / 2 + (inline ? 4 : 1)} textAnchor="middle" fontSize="11" fontWeight={600} style={{ fill: c.label }}>
          {z ? zoneName(z) : t(`techlog.cockpit.zone_${k}`)}
          {inline && c.n && (
            <tspan fontSize="9" fontWeight={400} style={{ fill: "rgb(var(--sf-muted))" }}>
              {"  ·  "}
              {c.n.differs > 0 ? t("techlog.cockpit.n_differ", { n: c.n.differs }) : t("techlog.cockpit.n_controls", { n: c.n.same + c.n.differs + c.n.none + c.n.skip })}
            </tspan>
          )}
        </text>
        {c.n && !inline && (
          <text x={x + w / 2} y={(ly ?? y + h / 2 + 1) + 13} textAnchor="middle" fontSize="9" style={{ fill: "rgb(var(--sf-muted))" }}>
            {c.n.differs > 0 ? t("techlog.cockpit.n_differ", { n: c.n.differs }) : t("techlog.cockpit.n_controls", { n: c.n.same + c.n.differs + c.n.none + c.n.skip })}
          </text>
        )}
      </g>
    );
  };
  return (
    <svg
      viewBox="0 0 420 250"
      className="mx-auto block h-auto w-full max-w-xl"
      style={{ maxHeight: "max(150px, calc(100vh - 660px))" }}
      aria-label={t("techlog.cockpit.schematic")}
    >
      {/* Morro y parabrisas */}
      <path d="M 40 112 Q 210 -6 380 112" fill="none" style={{ stroke: LINE }} strokeWidth="1.5" />
      <path d="M 70 104 Q 210 30 350 104" fill="rgba(56,189,248,0.05)" stroke="rgba(56,189,248,0.35)" strokeWidth="1" />
      <line x1="210" y1="49" x2="210" y2="104" stroke="rgba(56,189,248,0.35)" strokeWidth="1" />
      {/* Panel superior (en el techo, se dibuja arriba) */}
      <Zone k="overhead" x={140} y={8} w={140} h={44} />
      {/* Visera y panel principal */}
      <Zone k="glareshield" x={60} y={108} w={300} h={26} r={6} inline />
      <Zone k="main" x={44} y={140} w={332} h={42} r={6} ly={158} />
      {/* Pantallas del panel principal (decoración) */}
      {[64, 112, 270, 318].map((x) => (
        <rect key={x} x={x} y={164} width={38} height={14} rx={2} fill="none" style={{ stroke: LINE }} strokeWidth="1" />
      ))}
      {/* Pedestal entre los asientos */}
      <Zone k="pedestal" x={176} y={188} w={68} h={56} r={6} ly={212} />
      {/* Asientos */}
      {[78, 286].map((x) => (
        <g key={x} fill="none" style={{ stroke: LINE }} strokeWidth="1.2">
          <rect x={x} y={196} width={56} height={44} rx={10} />
          <rect x={x + 8} y={226} width={40} height={10} rx={4} />
        </g>
      ))}
    </svg>
  );
}
