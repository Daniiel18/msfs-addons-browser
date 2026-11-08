import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  Check,
  Layers,
  MapPin,
  Route,
  Package,
  Paintbrush,
  Pencil,
  Plane,
  RotateCcw,
  Save,
  ShieldCheck,
  Shield,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import {
  SIM_AIRCRAFT,
  focusKind,
  isPlane,
  parseStops,
  planSet,
  sceneriesFor,
  type DispatchFlight,
  type DispatchPlan,
  type NeedRow,
} from "../lib/dispatch";
import type { CommunityPackage, ContentSet } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useSimContentStore } from "../stores/useSimContentStore";
import { useLaunchStore } from "../stores/useLaunchStore";
import { useDispatchPlan, useDispatchStore, type SetsTab } from "../stores/useDispatchStore";
import { liverySearchUrl, openInFlightsimTab } from "../lib/flightsimTab";
import { usePreflightStore } from "../stores/usePreflightStore";
import { DetailModal } from "./DetailModal";

/** (03/11) Etapa 7 — revisión antes de cambiar addons, conjuntos y restaurar. */
export function DispatchHost() {
  const review = useDispatchStore((s) => s.review);
  const setsOpen = useDispatchStore((s) => s.setsOpen);
  const tripEditor = useDispatchStore((s) => s.tripEditor);
  return (
    <>
      {review?.kind === "flight" && <FlightReview flight={review.flight} seqId={review.seqId} />}
      {review?.kind === "set" && <SetReview setId={review.setId} />}
      {setsOpen && <SetsModal />}
      {tripEditor && <TripModal ofpAircraft={tripEditor.aircraft} />}
      <RestorePrompt />
    </>
  );
}

const isKept = (keep: string[], folder: string) => keep.some((f) => f.toLowerCase() === folder.toLowerCase());

/** Debe coincidir con `MAX_STOPS` de `dispatch.rs`. */
const MAX_TRIP_STOPS = 20;

const KIND_ORDER = ["airport", "aircraft", "livery", "other"] as const;
type Kind = (typeof KIND_ORDER)[number];
const KIND_ICON: Record<Kind, ReactNode> = {
  airport: <MapPin className="h-3.5 w-3.5" />,
  aircraft: <Plane className="h-3.5 w-3.5" />,
  livery: <Paintbrush className="h-3.5 w-3.5" />,
  other: <Package className="h-3.5 w-3.5" />,
};

function fmtDate(sqlite: string): string {
  const d = new Date(`${sqlite.replace(" ", "T")}Z`);
  if (Number.isNaN(d.getTime())) return sqlite;
  return d.toLocaleString(getActiveLocale(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

/** Botón «Nunca apagar» de un addon. */
function KeepButton({ folder, keep }: { folder: string; keep: string[] }) {
  const kept = isKept(keep, folder);
  return (
    <button
      type="button"
      aria-pressed={kept}
      onClick={() => void useDispatchStore.getState().toggleKeep(folder)}
      title={t(kept ? "dispatch.keep.remove" : "dispatch.keep.add")}
      aria-label={t(kept ? "dispatch.keep.remove" : "dispatch.keep.add")}
      className={cn(
        "shrink-0 rounded-md p-1 hover:bg-sf-raised",
        kept ? "text-brand-300" : "text-sf-muted hover:text-sf-text",
      )}
    >
      {kept ? <ShieldCheck className="h-3.5 w-3.5" /> : <Shield className="h-3.5 w-3.5" />}
    </button>
  );
}

function ChangeList({
  title,
  empty,
  items,
  keepable,
  keep,
}: {
  title: string;
  empty: string;
  items: CommunityPackage[];
  keepable?: boolean;
  keep: string[];
}) {
  const groups = KIND_ORDER.map((kind) => ({
    kind,
    list: items.filter((p) => (focusKind(p) ?? "other") === kind),
  })).filter((g) => g.list.length > 0);
  return (
    <section className="min-w-0 rounded-xl border border-sf-border bg-sf-surface">
      <h3 className="border-b border-sf-border px-3 py-2 text-xs font-semibold text-sf-text">{title}</h3>
      {groups.length === 0 ? (
        <p className="px-3 py-3 text-xs text-sf-muted">{empty}</p>
      ) : (
        <div className="max-h-[17rem] overflow-y-auto px-1.5 py-1.5">
          {groups.map(({ kind, list }) => (
            <div key={kind} className="mb-1.5 last:mb-0">
              <p className="flex items-center gap-1.5 px-1.5 py-1 text-[11px] font-medium text-sf-muted">
                {KIND_ICON[kind]}
                {t(`dispatch.kind.${kind}`)} ({list.length})
              </p>
              <ul>
                {list.map((p) => (
                  <li key={p.folderName} className="flex items-center gap-1 rounded-md px-1.5 py-0.5 hover:bg-sf-raised/60">
                    <span
                      className={cn(
                        "min-w-0 flex-1 truncate text-xs",
                        keepable && isKept(keep, p.folderName) ? "text-sf-muted line-through" : "text-sf-text",
                      )}
                      title={p.folderName}
                    >
                      {p.title}
                    </span>
                    {keepable && <KeepButton folder={p.folderName} keep={keep} />}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

const STATUS_CHIP: Record<NeedRow["status"], string> = {
  on: "border-emerald-500/30 text-emerald-300",
  turns_on: "border-sky-500/30 text-sky-300",
  none: "border-sf-border text-sf-muted",
  choose: "border-amber-500/40 text-amber-300",
  missing: "border-amber-500/40 text-amber-300",
  other: "border-amber-500/40 text-amber-300",
};

/** Lleva a la búsqueda de flightsim.to, que queda detrás de los modales. */
function searchLivery(registration: string) {
  useDispatchStore.getState().closeReview();
  usePreflightStore.getState().setOpen(false);
  openInFlightsimTab(liverySearchUrl(registration));
}

/** (05/11) Varios escenarios encendidos en un aeropuerto de la ruta. */
function ClashChoice({ row }: { row: NeedRow }) {
  const code = row.code as string;
  const choose = useDispatchStore.getState().setSceneryChoice;
  return (
    <div className="mt-2">
      <p className="mb-1 text-[11px] text-amber-200">{t("dispatch.clash.question")}</p>
      <div className="flex flex-wrap items-center gap-1.5">
        {row.clash?.map((p) => {
          const picked = row.status !== "choose" && row.pkg?.folderName === p.folderName;
          return (
            <button
              key={p.folderName}
              type="button"
              onClick={() => void choose(code, p.folderName, false)}
              className={cn(
                "inline-flex max-w-[18rem] items-center gap-1 truncate rounded-lg border px-2 py-1 text-xs",
                picked
                  ? "border-brand-500 bg-brand-500/10 text-sf-text"
                  : "border-sf-border text-sf-muted hover:text-sf-text",
              )}
            >
              {picked && <Check className="h-3 w-3 shrink-0 text-brand-400" />}
              <span className="truncate">{p.title}</span>
            </button>
          );
        })}
        <button
          type="button"
          onClick={() => void choose(code, null, true)}
          className="rounded-lg border border-sf-border px-2 py-1 text-xs text-sf-muted hover:text-sf-text"
        >
          {t("dispatch.clash.both")}
        </button>
      </div>
    </div>
  );
}

function NeedLine({ row, flight }: { row: NeedRow; flight: DispatchFlight | null }) {
  const packages = useCommunityStore((s) => s.packages);
  const pick = useDispatchStore((s) => (flight ? s.picks[flight.key] : undefined));
  const fromSim = useSimContentStore((s) => !!row.code && s.icaos.has(row.code.toUpperCase()));
  const isAircraft = row.role === "aircraft";
  const isLivery = row.role === "livery";
  const others = useMemo(
    () =>
      isAircraft
        ? packages
            .filter((p) => isPlane(p) && !row.candidates.includes(p))
            .sort((a, b) => a.title.localeCompare(b.title))
        : [],
    [isAircraft, packages, row.candidates],
  );
  const choose = (value: string) => flight && useDispatchStore.getState().pick(flight.key, value);
  const detailed = !isAircraft && !isLivery && !row.pkg && fromSim;
  const statusKey = row.status === "none" && (pick === SIM_AIRCRAFT || detailed) ? "sim" : row.status;

  return (
    <div className="rounded-xl border border-sf-border bg-sf-surface px-3 py-2">
      <div className="flex items-center gap-3">
        <span className="w-[4.5rem] shrink-0 text-[11px] text-sf-muted">{t(`dispatch.row.${row.role}`)}</span>
        <span className="w-16 shrink-0 truncate font-mono text-xs text-sf-text">{row.code ?? "—"}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-sf-text" title={row.pkg?.folderName}>
          {row.title ??
            row.pkg?.title ??
            t(
              isLivery
                ? row.status === "other"
                  ? "dispatch.row.other_livery"
                  : "dispatch.row.no_livery"
                : !isAircraft
                  ? detailed
                    ? "dispatch.row.detailed_scenery"
                    : "dispatch.row.default_scenery"
                  : pick === SIM_AIRCRAFT
                    ? "dispatch.row.sim_aircraft"
                    : "dispatch.row.no_aircraft",
            )}
        </span>
        {isLivery && row.status === "missing" && row.code && (
          <button
            type="button"
            onClick={() => searchLivery(row.code as string)}
            className="shrink-0 rounded-lg border border-sf-border px-2 py-0.5 text-[11px] font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("dispatch.row.search_flightsim")}
          </button>
        )}
        <span className={cn("shrink-0 rounded-md border px-1.5 py-0.5 text-[11px] font-medium", STATUS_CHIP[row.status])}>
          {t(`dispatch.status.${statusKey}`)}
        </span>
      </div>
      {isAircraft && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {row.candidates.map((p) => (
            <button
              key={p.folderName}
              type="button"
              onClick={() => choose(p.folderName)}
              className={cn(
                "inline-flex max-w-[18rem] items-center gap-1 truncate rounded-lg border px-2 py-1 text-xs",
                row.pkg?.folderName === p.folderName
                  ? "border-brand-500 bg-brand-500/10 text-sf-text"
                  : "border-sf-border text-sf-muted hover:text-sf-text",
              )}
            >
              {row.pkg?.folderName === p.folderName && <Check className="h-3 w-3 shrink-0 text-brand-400" />}
              <span className="truncate">{p.title}</span>
            </button>
          ))}
          <button
            type="button"
            onClick={() => choose(SIM_AIRCRAFT)}
            title={t("dispatch.row.sim_hint")}
            className={cn(
              "rounded-lg border px-2 py-1 text-xs",
              pick === SIM_AIRCRAFT
                ? "border-brand-500 bg-brand-500/10 text-sf-text"
                : "border-sf-border text-sf-muted hover:text-sf-text",
            )}
          >
            {t("dispatch.row.sim_aircraft")}
          </button>
          {others.length > 0 && (
            <select
              value=""
              onChange={(e) => e.target.value && choose(e.target.value)}
              aria-label={t("dispatch.row.other")}
              className="max-w-[12rem] rounded-lg border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-muted"
            >
              <option value="">{t("dispatch.row.other")}</option>
              {others.map((p) => (
                <option key={p.folderName} value={p.folderName}>
                  {p.title}
                </option>
              ))}
            </select>
          )}
        </div>
      )}
      {row.clash && row.code && <ClashChoice row={row} />}
    </div>
  );
}

function ReviewBody({ plan, flight }: { plan: DispatchPlan; flight: DispatchFlight | null }) {
  const keep = useDispatchStore((s) => s.keep);
  const simRunning = useDispatchStore((s) => s.simRunning);
  const changes = plan.on.length + plan.off.length;
  return (
    <div className="space-y-3">
      {simRunning && changes > 0 && (
        <p className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
          <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {t("dispatch.review.sim_open")}
        </p>
      )}
      {plan.rows.length > 0 && (
        <div className="space-y-1.5">
          {plan.rows.map((r, i) => (
            <NeedLine key={`${r.role}-${i}`} row={r} flight={flight} />
          ))}
        </div>
      )}
      <div className="grid gap-3 md:grid-cols-2">
        <ChangeList
          title={t("dispatch.review.turn_on", { n: plan.on.length })}
          empty={t("dispatch.review.nothing_on")}
          items={plan.on}
          keep={keep}
        />
        <ChangeList
          title={t("dispatch.review.turn_off", { n: plan.off.length })}
          empty={t("dispatch.review.nothing_off")}
          items={[...plan.off, ...plan.kept]}
          keepable
          keep={keep}
        />
      </div>
      {plan.off.length + plan.kept.length > 0 && (
        <p className="text-[11px] leading-snug text-sf-muted">{t("dispatch.review.keep_hint")}</p>
      )}
      {plan.skipped.length > 0 && (
        <p className="text-[11px] leading-snug text-sf-muted">
          {t("dispatch.review.skipped", { names: plan.skipped.map((p) => p.title).join(", ") })}
        </p>
      )}
      {plan.missing > 0 && (
        <p className="text-[11px] leading-snug text-sf-muted">{t("dispatch.review.missing", { n: plan.missing })}</p>
      )}
    </div>
  );
}

function FooterButtons({
  onCancel,
  onConfirm,
  confirm,
  disabled,
}: {
  onCancel: () => void;
  onConfirm?: () => void;
  confirm?: string;
  disabled?: boolean;
}) {
  return (
    <>
      <button
        type="button"
        onClick={onCancel}
        className="rounded-xl border border-sf-border px-4 py-2 text-xs font-medium text-sf-text hover:bg-sf-raised"
      >
        {t(onConfirm ? "dispatch.review.cancel" : "dispatch.review.close")}
      </button>
      {onConfirm && (
        <button
          type="button"
          disabled={disabled}
          onClick={onConfirm}
          className="rounded-xl bg-brand-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {confirm}
        </button>
      )}
    </>
  );
}

function FlightReview({ flight: ofpFlight, seqId }: { flight: DispatchFlight | null; seqId: number | null }) {
  const plan = useDispatchPlan(ofpFlight);
  const { flight } = plan;
  const close = () => useDispatchStore.getState().closeReview();
  const launch = async () => {
    close();
    if (seqId !== null) await useLaunchStore.getState().launch(seqId, false, plan.changes ?? undefined);
  };
  const title = plan.set
    ? t("dispatch.review.title_set", { name: plan.set.name })
    : plan.trip
      ? t("dispatch.line.trip", { n: plan.trip.stops.length })
      : t(`dispatch.mode.${plan.mode === "set" ? "just" : plan.mode}`);
  return (
    <DetailModal
      title={title}
      subtitle={
        plan.trip
          ? plan.trip.stops.join(" → ")
          : flight
            ? `${flight.origin ?? "····"} → ${flight.destination ?? "····"}`
            : undefined
      }
      icon={<Layers className="h-4 w-4" />}
      onClose={close}
      zClass="z-[64]"
      footer={
        <FooterButtons
          onCancel={close}
          onConfirm={seqId !== null ? () => void launch() : undefined}
          disabled={plan.needsChoice}
          confirm={
            plan.needsChoice
              ? t(plan.rows.some((r) => r.clash && r.status === "choose") ? "dispatch.clash.choose_first" : "dispatch.review.choose_first")
              : plan.off.length > 0
                ? t("dispatch.review.off_and_launch", { n: plan.off.length })
                : t("dispatch.review.launch")
          }
        />
      }
    >
      <ReviewBody plan={plan} flight={flight} />
    </DetailModal>
  );
}

function SetReview({ setId }: { setId: number }) {
  const packages = useCommunityStore((s) => s.packages);
  const keep = useDispatchStore((s) => s.keep);
  const target = useDispatchStore((s) => s.sets.find((x) => x.id === setId));
  const plan = useMemo(() => (target ? planSet(target, packages, keep) : null), [target, packages, keep]);
  const close = () => useDispatchStore.getState().closeReview();
  if (!target || !plan) return null;
  const apply = async () => {
    close();
    await useDispatchStore.getState().applySet(target);
  };
  return (
    <DetailModal
      title={t("dispatch.review.title_set", { name: target.name })}
      subtitle={fmtDate(target.updatedAt)}
      icon={<Layers className="h-4 w-4" />}
      onClose={close}
      zClass="z-[64]"
      footer={
        <FooterButtons
          onCancel={close}
          onConfirm={() => void apply()}
          disabled={plan.on.length + plan.off.length === 0}
          confirm={t(target.kind === "restore" ? "dispatch.review.restore" : "dispatch.review.apply")}
        />
      }
    >
      <ReviewBody plan={plan} flight={null} />
    </DetailModal>
  );
}

function SetRow({ set, pending }: { set: ContentSet; pending: boolean }) {
  const packages = useCommunityStore((s) => s.packages);
  const keep = useDispatchStore((s) => s.keep);
  const plan = useMemo(() => planSet(set, packages, keep), [set, packages, keep]);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(set.name);
  const [sure, setSure] = useState(false);
  const store = useDispatchStore.getState();
  const isUser = set.kind === "user";
  const same = plan.on.length + plan.off.length === 0;
  const commit = () => {
    setEditing(false);
    if (name.trim() && name.trim() !== set.name) void store.renameSet(set.id, name);
    else setName(set.name);
  };

  return (
    <li className="flex items-center gap-3 rounded-xl border border-sf-border bg-sf-surface px-3 py-2">
      <div className="min-w-0 flex-1">
        {editing ? (
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setName(set.name);
                setEditing(false);
              }
            }}
            className="w-full rounded-md border border-sf-border bg-sf-raised px-2 py-0.5 text-sm text-sf-text outline-none focus:border-brand-500"
          />
        ) : (
          <p className="truncate text-sm font-medium text-sf-text" title={set.name}>
            {set.name}
            {pending && (
              <span className="ml-2 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-300">
                {t("dispatch.sets.pending")}
              </span>
            )}
          </p>
        )}
        <p className="truncate text-[11px] text-sf-muted">
          {t("dispatch.sets.counts", { on: set.enabled.length, total: set.known.length })} · {fmtDate(set.updatedAt)} ·{" "}
          {same
            ? t("dispatch.sets.same")
            : t("dispatch.sets.diff", { on: plan.on.length, off: plan.off.length })}
        </p>
      </div>
      {sure ? (
        <>
          <span className="text-xs text-sf-muted">{t("dispatch.sets.delete_q")}</span>
          <button
            type="button"
            onClick={() => void store.deleteSet(set.id)}
            className="rounded-lg bg-rose-500/90 px-2.5 py-1 text-xs font-semibold text-white hover:bg-rose-500"
          >
            {t("dispatch.sets.delete")}
          </button>
          <button
            type="button"
            onClick={() => setSure(false)}
            className="rounded-lg border border-sf-border px-2.5 py-1 text-xs text-sf-text hover:bg-sf-raised"
          >
            {t("dispatch.review.cancel")}
          </button>
        </>
      ) : (
        <>
          <button
            type="button"
            disabled={same}
            onClick={() => store.openReview({ kind: "set", setId: set.id })}
            className="rounded-lg bg-brand-500 px-2.5 py-1 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t(isUser ? "dispatch.sets.apply" : "dispatch.sets.restore")}
          </button>
          {isUser && (
            <>
              <button
                type="button"
                onClick={() => void store.updateSet(set.id)}
                title={t("dispatch.sets.update_hint")}
                aria-label={t("dispatch.sets.update_hint")}
                className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
              >
                <Save className="h-4 w-4" />
              </button>
              <button
                type="button"
                onClick={() => setEditing(true)}
                title={t("dispatch.sets.rename")}
                aria-label={t("dispatch.sets.rename")}
                className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
              >
                <Pencil className="h-4 w-4" />
              </button>
            </>
          )}
          <button
            type="button"
            onClick={() => setSure(true)}
            title={t("dispatch.sets.delete")}
            aria-label={t("dispatch.sets.delete")}
            className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-rose-300"
          >
            <Trash2 className="h-4 w-4" />
          </button>
        </>
      )}
    </li>
  );
}

function KeepTab() {
  const packages = useCommunityStore((s) => s.packages);
  const keep = useDispatchStore((s) => s.keep);
  const [q, setQ] = useState("");
  const list = useMemo(() => {
    const term = q.trim().toLowerCase();
    return packages
      .filter((p) => focusKind(p) !== null)
      .filter((p) => !term || `${p.title} ${p.folderName} ${p.icao ?? ""}`.toLowerCase().includes(term))
      .sort(
        (a, b) =>
          Number(isKept(keep, b.folderName)) - Number(isKept(keep, a.folderName)) || a.title.localeCompare(b.title),
      );
  }, [packages, keep, q]);
  return (
    <div className="space-y-3">
      <p className="text-xs leading-relaxed text-sf-muted">{t("dispatch.keep.explain")}</p>
      <input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={t("dispatch.keep.search")}
        aria-label={t("dispatch.keep.search")}
        className="w-full rounded-xl border border-sf-border bg-sf-surface px-3 py-2 text-sm text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500"
      />
      <ul className="grid gap-1 md:grid-cols-2">
        {list.map((p) => (
          <li
            key={p.folderName}
            className="flex items-center gap-2 rounded-lg border border-sf-border bg-sf-surface px-2.5 py-1.5"
          >
            <span className="shrink-0 text-sf-muted">{KIND_ICON[focusKind(p) ?? "other"]}</span>
            <span className="min-w-0 flex-1 truncate text-xs text-sf-text" title={p.folderName}>
              {p.title}
            </span>
            <KeepButton folder={p.folderName} keep={keep} />
          </li>
        ))}
      </ul>
    </div>
  );
}

function SetsModal() {
  const sets = useDispatchStore((s) => s.sets);
  const pending = useDispatchStore((s) => s.pendingRestore);
  const keep = useDispatchStore((s) => s.keep);
  const tab = useDispatchStore((s) => s.setsTab);
  const onCount = useCommunityStore((s) => s.packages.filter((p) => p.enabled !== false && p.hasManifest !== false).length);
  const [name, setName] = useState("");
  const user = sets.filter((s) => s.kind === "user");
  const points = sets.filter((s) => s.kind === "restore");
  const tabs: { id: SetsTab; label: string; icon: ReactNode; badge?: number }[] = [
    { id: "sets", label: t("dispatch.sets.tab_sets"), icon: <Layers className="h-3.5 w-3.5" /> },
    {
      id: "restore",
      label: t("dispatch.sets.tab_restore"),
      icon: <RotateCcw className="h-3.5 w-3.5" />,
      badge: pending !== null ? 1 : undefined,
    },
    { id: "keep", label: t("dispatch.sets.tab_keep"), icon: <ShieldCheck className="h-3.5 w-3.5" /> },
  ];
  const save = async () => {
    if (!name.trim()) return;
    if (await useDispatchStore.getState().saveSet(name)) setName("");
  };

  return (
    <DetailModal
      title={t("dispatch.sets.title")}
      subtitle={t("dispatch.sets.subtitle", { n: keep.length })}
      icon={<Layers className="h-4 w-4" />}
      tabs={tabs}
      tab={tab}
      onTab={(id) => useDispatchStore.setState({ setsTab: id as SetsTab })}
      onClose={() => useDispatchStore.getState().closeSets()}
      zClass="z-[62]"
    >
      {tab === "sets" && (
        <div className="space-y-3">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
            className="flex gap-2"
          >
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("dispatch.sets.name_placeholder")}
              aria-label={t("dispatch.sets.name_placeholder")}
              className="min-w-0 flex-1 rounded-xl border border-sf-border bg-sf-surface px-3 py-2 text-sm text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500"
            />
            <button
              type="submit"
              disabled={!name.trim()}
              className="shrink-0 rounded-xl bg-brand-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {t("dispatch.sets.save", { n: onCount })}
            </button>
          </form>
          {user.length === 0 ? (
            <p className="py-6 text-center text-xs text-sf-muted">{t("dispatch.sets.empty")}</p>
          ) : (
            <ul className="space-y-1.5">
              {user.map((s) => (
                <SetRow key={s.id} set={s} pending={false} />
              ))}
            </ul>
          )}
        </div>
      )}
      {tab === "restore" && (
        <div className="space-y-3">
          <p className="text-xs leading-relaxed text-sf-muted">{t("dispatch.sets.restore_explain")}</p>
          {points.length === 0 ? (
            <p className="py-6 text-center text-xs text-sf-muted">{t("dispatch.sets.restore_empty")}</p>
          ) : (
            <ul className="space-y-1.5">
              {points.map((s) => (
                <SetRow key={s.id} set={s} pending={s.id === pending} />
              ))}
            </ul>
          )}
        </div>
      )}
      {tab === "keep" && <KeepTab />}
    </DetailModal>
  );
}

/** (04/11) Etapa 9 — viaje de varias paradas: códigos y tipo de avión. */
function TripModal({ ofpAircraft }: { ofpAircraft: string | null }) {
  const trip = useDispatchStore((s) => s.trip);
  const packages = useCommunityStore((s) => s.packages);
  const [text, setText] = useState(trip?.stops.join(" ") ?? "");
  const [aircraft, setAircraft] = useState(trip?.aircraft ?? ofpAircraft ?? "");
  const store = useDispatchStore.getState();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      useDispatchStore.getState().closeTrip();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const stops = useMemo(() => parseStops(text), [text]);
  const hasScenery = (code: string) => sceneriesFor(code, packages).length > 0;
  const unique = [...new Set(stops)];
  const covered = unique.filter(hasScenery).length;
  const tooMany = stops.length > MAX_TRIP_STOPS;
  const valid = stops.length >= 2 && !tooMany;
  const save = () => {
    if (valid) void store.saveTrip(stops, aircraft.trim() || null);
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("dispatch.trip.title")}
      onClick={store.closeTrip}
      className="fixed inset-0 z-[64] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-[min(520px,calc(100vw-2rem))] space-y-3 rounded-2xl border border-sf-border bg-sf-bg p-5 shadow-2xl"
      >
        <h2 className="flex items-center gap-2 text-sm font-semibold text-sf-text">
          <Route className="h-4 w-4 text-brand-400" />
          {t("dispatch.trip.title")}
        </h2>
        <label className="block text-xs text-sf-muted">
          {t("dispatch.trip.stops")}
          <textarea
            autoFocus
            rows={2}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="KJFK CYYZ CYOW KBOS"
            className="mt-1 w-full resize-none rounded-xl border border-sf-border bg-sf-surface px-3 py-2 font-mono text-sm uppercase text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500"
          />
        </label>
        {stops.length > 0 && (
          <div className="space-y-1.5">
            <ul className="flex flex-wrap gap-1.5">
              {stops.map((code, i) => (
                <li
                  key={`${code}-${i}`}
                  title={t(hasScenery(code) ? "dispatch.trip.has_scenery" : "dispatch.trip.no_scenery")}
                  className={cn(
                    "rounded-md border px-1.5 py-0.5 font-mono text-[11px]",
                    hasScenery(code)
                      ? "border-emerald-500/30 text-emerald-300"
                      : "border-sf-border text-sf-muted",
                  )}
                >
                  {code}
                </li>
              ))}
            </ul>
            <p className="text-[11px] text-sf-muted">
              {t("dispatch.trip.covered", { n: covered, total: unique.length })}
            </p>
          </div>
        )}
        <label className="block text-xs text-sf-muted">
          {t("dispatch.trip.aircraft")}
          <input
            value={aircraft}
            onChange={(e) => setAircraft(e.target.value.toUpperCase())}
            maxLength={4}
            placeholder="A320"
            className="mt-1 block w-28 rounded-xl border border-sf-border bg-sf-surface px-3 py-2 font-mono text-sm text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500"
          />
        </label>
        <p className={cn("text-[11px] leading-snug", tooMany ? "font-medium text-amber-300" : "text-sf-muted")}>
          {t(tooMany ? "dispatch.trip.too_many" : "dispatch.trip.hint", { n: MAX_TRIP_STOPS })}
        </p>
        <div className="flex items-center justify-end gap-2">
          {trip && (
            <button
              type="button"
              onClick={() => void store.clearTrip()}
              className="mr-auto rounded-xl px-3 py-2 text-xs font-medium text-rose-300 hover:bg-sf-raised"
            >
              {t("dispatch.trip.remove")}
            </button>
          )}
          <button
            type="button"
            onClick={store.closeTrip}
            className="rounded-xl border border-sf-border px-4 py-2 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("dispatch.review.cancel")}
          </button>
          <button
            type="button"
            disabled={!valid}
            onClick={save}
            className="rounded-xl bg-brand-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t("dispatch.trip.save")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Aviso al cerrar MSFS con addons apagados. */
function RestorePrompt() {
  const open = useDispatchStore((s) => s.restorePrompt);
  const pending = useDispatchStore((s) => s.pendingRestore);
  const [busy, setBusy] = useState(false);
  if (!open || pending === null) return null;
  const store = useDispatchStore.getState();
  const restore = async () => {
    setBusy(true);
    await store.restore();
    setBusy(false);
  };
  return (
    <div
      role="alertdialog"
      aria-label={t("dispatch.prompt.title")}
      className="fixed bottom-4 left-1/2 z-[66] flex w-[min(420px,calc(100vw-2rem))] -translate-x-1/2 items-center gap-3 rounded-2xl border border-sf-border bg-sf-surface p-4 shadow-2xl"
    >
      <RotateCcw className="h-5 w-5 shrink-0 text-brand-400" />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-sf-text">{t("dispatch.prompt.title")}</p>
        <p className="text-xs leading-snug text-sf-muted">{t("dispatch.prompt.body")}</p>
      </div>
      <button
        type="button"
        onClick={store.dismissRestorePrompt}
        className="shrink-0 rounded-lg border border-sf-border px-2.5 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
      >
        {t("dispatch.prompt.later")}
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={() => void restore()}
        className="shrink-0 rounded-lg bg-brand-500 px-2.5 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-60"
      >
        {t("dispatch.prompt.restore")}
      </button>
    </div>
  );
}
