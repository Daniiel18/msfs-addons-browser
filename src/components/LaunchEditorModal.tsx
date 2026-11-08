import { useEffect, useMemo, useRef, useState, type InputHTMLAttributes, type ReactNode } from "react";
import { Reorder, motion, useDragControls } from "framer-motion";
import {
  AppWindow,
  ChevronDown,
  Clock,
  Eye,
  FolderOpen,
  Globe,
  GripVertical,
  Pencil,
  Plane,
  Play,
  Plus,
  Rocket,
  ShieldAlert,
  TerminalSquare,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { appStep, folderStep, isWebUrl, launchError, scriptStep, simStep, stepFromApp, waitLabel, webStep } from "../lib/launchFlow";
import type { LaunchClose, LaunchOpen, LaunchScriptPermission, LaunchSequence, LaunchStep, LaunchStepKind } from "../lib/types";
import { useLaunchStore } from "../stores/useLaunchStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useToastStore } from "../stores/useToastStore";

const SCRIPT_EXTENSIONS = ["bat", "cmd", "ps1"];

/** Monta el editor cuando está abierto, y el permiso de los scripts. */
export function LaunchEditorHost() {
  const open = useLaunchStore((s) => s.editorOpen);
  const ask = useLaunchStore((s) => s.scriptAsk);
  return (
    <>
      {open && <LaunchEditorModal />}
      {ask && <ScriptPermissionModal scripts={ask.scripts} />}
    </>
  );
}

/** (07/11) Etapa F: un script corre lo que diga su archivo, así que se pregunta. */
function ScriptPermissionModal({ scripts }: { scripts: LaunchScriptPermission[] }) {
  const { allowScripts, denyScripts } = useLaunchStore.getState();
  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm">
      <div className="w-[min(560px,100%)] space-y-3 rounded-2xl border border-sf-border bg-sf-surface p-5 shadow-2xl">
        <div className="flex items-start gap-2">
          <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-300" />
          <div className="min-w-0">
            <p className="text-sm font-semibold text-sf-text">{t("launch.script.title")}</p>
            <p className="mt-0.5 text-[11px] text-sf-muted">{t("launch.script.hint")}</p>
          </div>
        </div>
        <ul className="max-h-[50vh] space-y-2 overflow-y-auto">
          {scripts.map((s) => (
            <li key={s.hash} className="rounded-lg border border-sf-border bg-sf-bg/40 p-2">
              <p className="truncate text-xs font-medium text-sf-text">{s.name}</p>
              <p className="truncate font-mono text-[10px] text-sf-muted" title={s.path}>
                {s.path}
              </p>
              <pre className="mt-1.5 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-sf-raised/60 p-2 font-mono text-[10px] text-sf-text">
                {s.preview}
              </pre>
            </li>
          ))}
        </ul>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={denyScripts}
            className="rounded-lg border border-sf-border px-3 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("launch.script.deny")}
          </button>
          <button
            type="button"
            onClick={() => void allowScripts()}
            className="rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-brand-400"
          >
            {t("launch.script.allow")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Texto que se guarda al salir del campo (o con Enter), no a cada tecla. */
function CommitInput({
  value,
  onCommit,
  ...rest
}: { value: string; onCommit: (v: string) => void } & Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange">) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      {...rest}
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}

const KIND_ICON: Record<LaunchStepKind, ReactNode> = {
  app: <AppWindow className="h-4 w-4 text-sf-muted" />,
  web: <Globe className="h-4 w-4 text-sky-400" />,
  folder: <FolderOpen className="h-4 w-4 text-amber-300" />,
  sim: <Plane className="h-4 w-4 text-brand-400" />,
  script: <TerminalSquare className="h-4 w-4 text-violet-300" />,
};

const OPEN_MODES: LaunchOpen[] = ["normal", "minimized", "admin"];
const CLOSE_MODES: LaunchClose[] = ["stay", "exit"];

const SELECT = "w-full rounded-lg border border-sf-border bg-sf-bg px-2 py-1 text-[11px] text-sf-text outline-none focus:border-brand-500/60";
const PILL = "block w-full truncate rounded-lg bg-sf-raised px-2 py-1 text-center text-[11px] text-sf-muted";
const FIELD = "w-full rounded-lg border border-sf-border bg-sf-bg px-2.5 py-1.5 text-xs text-sf-text outline-none placeholder:text-sf-muted/60 focus:border-brand-500/60";

/** Cómo se abre un paso, en palabras. */
function openLabel(s: LaunchStep): string {
  if (s.kind === "web") return t("launch.open.browser");
  if (s.kind === "folder") return t("launch.open.explorer");
  if (s.kind === "sim") return t("launch.kind.sim");
  if (s.kind === "script") return t("launch.open.script");
  return t(`launch.open.${s.open}`);
}

/**
 * (02/11) Etapa 6 — editor de las secuencias de arranque: a la izquierda la
 * lista; a la derecha sus aviones y sus pasos, en orden. Los cambios se
 * guardan solos.
 */
function LaunchEditorModal() {
  const sequences = useLaunchStore((s) => s.sequences);
  const apps = useLaunchStore((s) => s.apps);
  const sim = useLaunchStore((s) => s.sim);
  const focus = useLaunchStore((s) => s.editorFocus);
  const running = useLaunchStore((s) => s.run?.state === "running");

  const [selectedId, setSelectedId] = useState<number | null>(focus);
  const [draft, setDraft] = useState<LaunchSequence | null>(null);
  const [preview, setPreview] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Guardado automático: `pending` es lo que falta por guardar.
  const draftRef = useRef<LaunchSequence | null>(null);
  draftRef.current = draft;
  const pending = useRef<LaunchSequence | null>(null);
  const timer = useRef<number | null>(null);
  const edits = useRef(0);

  const flush = async () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const seq = pending.current;
    pending.current = null;
    if (!seq || !seq.name.trim()) return;
    const at = edits.current;
    await useLaunchStore.getState().save(seq);
    // Sin cambios nuevos: toma lo que normalizó el back.
    if (edits.current === at && draftRef.current?.id === seq.id) {
      const fresh = useLaunchStore.getState().sequences.find((s) => s.id === seq.id);
      if (fresh) setDraft(fresh);
    }
  };

  const change = (fn: (d: LaunchSequence) => LaunchSequence) => {
    const d = draftRef.current;
    if (!d) return;
    const next = fn(d);
    draftRef.current = next;
    setDraft(next);
    pending.current = next;
    edits.current += 1;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void flush(), 500);
  };

  const setStep = (id: string, patch: Partial<LaunchStep>) =>
    change((d) => ({ ...d, steps: d.steps.map((s) => (s.id === id ? { ...s, ...patch } : s)) }));
  const addStep = (step: LaunchStep) => change((d) => ({ ...d, steps: [...d.steps, step] }));

  // La secuencia elegida (o la primera) pasa a ser el borrador.
  useEffect(() => {
    if (draft && draft.id === selectedId) return;
    const seq = sequences.find((s) => s.id === selectedId) ?? sequences[0] ?? null;
    setDraft(seq);
    if (seq && seq.id !== selectedId) setSelectedId(seq.id);
    setExpanded(null);
    setConfirmDelete(false);
    setPreview(false);
  }, [sequences, selectedId, draft]);

  const select = async (id: number) => {
    await flush();
    setSelectedId(id);
    setDraft(null);
  };

  const close = async () => {
    await flush();
    useLaunchStore.getState().closeEditor();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const create = async () => {
    await flush();
    const id = await useLaunchStore.getState().save({
      id: 0,
      name: t("launch.editor.new_name"),
      aircraft: [],
      isDefault: sequences.length === 0,
      steps: sim?.available ? [{ ...simStep(), waitSeconds: 0 }] : [],
    });
    if (id !== null) {
      setSelectedId(id);
      setDraft(null);
    }
  };

  const remove = async () => {
    if (!draft) return;
    const id = draft.id;
    pending.current = null;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    await useLaunchStore.getState().remove(id);
    setDraft(null);
    setSelectedId(null);
  };

  const testAll = async () => {
    if (!draft || running) return;
    await flush();
    await useLaunchStore.getState().launch(draft.id, true);
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.12 }}
      onClick={() => void close()}
      className="fixed inset-0 z-[62] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16 }}
        onClick={(e) => e.stopPropagation()}
        className="flex h-[min(680px,calc(100vh-3rem))] w-[min(980px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl ring-1 ring-sf-border"
      >
        <header className="flex items-center gap-3 border-b border-sf-border px-5 py-3">
          <Rocket className="h-4 w-4 text-brand-400" />
          <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-sf-text">{t("launch.editor.title")}</h2>
          {draft && draft.steps.length > 0 && (
            <>
              <button
                type="button"
                onClick={() => setPreview((p) => !p)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-sf-border px-2.5 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
              >
                {preview ? <Pencil className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                {t(preview ? "launch.editor.edit" : "launch.editor.preview")}
              </button>
              <button
                type="button"
                onClick={() => void testAll()}
                title={t("launch.editor.test_all_hint")}
                className="inline-flex items-center gap-1.5 rounded-lg border border-sf-border px-2.5 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
              >
                <Play className="h-3.5 w-3.5" />
                {t("launch.editor.test_all")}
              </button>
            </>
          )}
          <button
            type="button"
            onClick={() => void close()}
            aria-label={t("launch.run.dismiss")}
            className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="grid min-h-0 flex-1 grid-cols-[220px_minmax(0,1fr)]">
          {/* Secuencias */}
          <aside className="flex min-h-0 flex-col border-r border-sf-border">
            <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
              {sequences.map((s) => {
                const active = s.id === draft?.id;
                const shown = active && draft ? draft : s;
                return (
                  <li key={s.id}>
                    <button
                      type="button"
                      onClick={() => void select(s.id)}
                      className={cn(
                        "w-full rounded-lg px-2.5 py-2 text-left",
                        active ? "bg-brand-500/15 ring-1 ring-brand-500/30" : "hover:bg-sf-raised",
                      )}
                    >
                      <span className="block truncate text-xs font-semibold text-sf-text">{shown.name || "—"}</span>
                      <span className="mt-0.5 block truncate text-[11px] text-sf-muted">
                        {shown.aircraft.length > 0
                          ? shown.aircraft.join(" · ")
                          : shown.isDefault
                            ? t("launch.menu.default")
                            : t("launch.menu.no_aircraft")}
                      </span>
                      <span className="block text-[11px] text-sf-muted/80">
                        {t("launch.editor.steps_count", { n: String(shown.steps.length) })}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
            <div className="border-t border-sf-border p-2">
              <button
                type="button"
                onClick={() => void create()}
                className="inline-flex w-full items-center justify-center gap-1.5 rounded-lg border border-dashed border-sf-border px-2.5 py-2 text-xs font-medium text-sf-muted hover:border-brand-500/40 hover:text-sf-text"
              >
                <Plus className="h-3.5 w-3.5" />
                {t("launch.editor.new")}
              </button>
            </div>
          </aside>

          {/* La secuencia elegida */}
          {draft ? (
            <div className="flex min-h-0 flex-col">
              <SequenceHeader
                key={draft.id}
                draft={draft}
                sequences={sequences}
                confirmDelete={confirmDelete}
                onConfirmDelete={setConfirmDelete}
                onDelete={() => void remove()}
                onChange={change}
              />
              {sim?.testEnv && (
                <p className="border-b border-sf-border bg-amber-500/10 px-5 py-1.5 text-[11px] text-amber-300">
                  {t("launch.editor.test_env")}
                </p>
              )}
              {preview ? (
                <Preview draft={draft} simLabel={sim?.label ?? ""} />
              ) : (
                <>
                  <div className="min-h-0 flex-1 overflow-y-auto px-5 py-2">
                    {draft.steps.length === 0 ? (
                      <p className="py-10 text-center text-xs text-sf-muted">{t("launch.editor.no_steps")}</p>
                    ) : (
                      <Reorder.Group
                        axis="y"
                        values={draft.steps}
                        onReorder={(steps) => change((d) => ({ ...d, steps }))}
                        className="list-none"
                      >
                        {draft.steps.map((s, i) => (
                          <StepRow
                            key={s.id}
                            step={s}
                            first={i === 0}
                            missing={draft.missing?.includes(s.id) ?? false}
                            simLabel={sim?.available ? sim.label : t("launch.editor.no_sim")}
                            simMissing={!sim?.available && !sim?.testEnv}
                            expanded={expanded === s.id}
                            onToggle={() => setExpanded((e) => (e === s.id ? null : s.id))}
                            onChange={(patch) => setStep(s.id, patch)}
                            onRemove={() => change((d) => ({ ...d, steps: d.steps.filter((x) => x.id !== s.id) }))}
                          />
                        ))}
                      </Reorder.Group>
                    )}
                  </div>
                  <AddBar
                    draft={draft}
                    apps={apps}
                    simAvailable={!!sim?.available || !!sim?.testEnv}
                    onAdd={addStep}
                  />
                </>
              )}
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center gap-3 p-10 text-center">
              {sequences.length === 0 && (
                <>
                  <Rocket className="h-8 w-8 text-sf-muted" />
                  <p className="text-sm font-semibold text-sf-text">{t("launch.editor.empty_title")}</p>
                  <p className="max-w-sm text-xs text-sf-muted">{t("launch.editor.empty_body")}</p>
                  <button
                    type="button"
                    onClick={() => void create()}
                    className="inline-flex items-center gap-1.5 rounded-xl bg-brand-500 px-4 py-2 text-sm font-semibold text-slate-950 hover:bg-brand-400"
                  >
                    <Plus className="h-4 w-4" />
                    {t("launch.editor.new")}
                  </button>
                </>
              )}
            </div>
          )}
        </div>
      </motion.div>
    </motion.div>
  );
}

function SequenceHeader({
  draft,
  sequences,
  confirmDelete,
  onConfirmDelete,
  onDelete,
  onChange,
}: {
  draft: LaunchSequence;
  sequences: LaunchSequence[];
  confirmDelete: boolean;
  onConfirmDelete: (v: boolean) => void;
  onDelete: () => void;
  onChange: (fn: (d: LaunchSequence) => LaunchSequence) => void;
}) {
  const ofps = useSimBriefStore((s) => s.flights);
  const [typed, setTyped] = useState("");
  const suggestions = useMemo(() => {
    const set = new Set<string>();
    for (const f of ofps) if (f.aircraftIcao) set.add(f.aircraftIcao.toUpperCase());
    for (const s of sequences) for (const a of s.aircraft) set.add(a);
    return [...set].filter((a) => !draft.aircraft.includes(a)).sort();
  }, [ofps, sequences, draft.aircraft]);

  const addTypes = (raw: string) => {
    const types = raw
      .split(/[\s,;·]+/)
      .map((a) => a.trim().toUpperCase())
      .filter((a) => /^[A-Z0-9]{2,4}$/.test(a) && !draft.aircraft.includes(a));
    setTyped("");
    if (types.length === 0) return;
    for (const a of types) {
      const other = sequences.find((s) => s.id !== draft.id && s.aircraft.includes(a));
      if (other) {
        useToastStore.getState().push({
          kind: "info",
          title: t("launch.editor.type_moved", { type: a, name: other.name }),
          ttlMs: 3600,
        });
      }
    }
    onChange((d) => ({ ...d, aircraft: [...d.aircraft, ...types] }));
  };

  return (
    <div className="space-y-2 border-b border-sf-border px-5 py-3">
      <div className="flex items-center gap-3">
        <CommitInput
          value={draft.name}
          onCommit={(name) => {
            if (name.trim()) onChange((d) => ({ ...d, name }));
            else useToastStore.getState().push({ kind: "error", title: launchError("name_required"), ttlMs: 2600 });
          }}
          aria-label={t("launch.editor.name")}
          maxLength={60}
          className="min-w-0 flex-1 rounded-lg border border-transparent bg-transparent px-2 py-1 text-sm font-semibold text-sf-text outline-none hover:border-sf-border focus:border-brand-500/60"
        />
        <label className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 text-[11px] text-sf-muted hover:text-sf-text">
          <input
            type="checkbox"
            className="h-3 w-3 accent-brand-500"
            checked={draft.isDefault}
            onChange={(e) => onChange((d) => ({ ...d, isDefault: e.target.checked }))}
          />
          {t("launch.editor.default")}
        </label>
        {confirmDelete ? (
          <span className="inline-flex shrink-0 items-center gap-1.5">
            <button
              type="button"
              onClick={onDelete}
              className="rounded-md bg-rose-500/80 px-2 py-1 text-[11px] font-semibold text-white hover:bg-rose-500"
            >
              {t("launch.editor.delete_confirm", { name: draft.name })}
            </button>
            <button
              type="button"
              onClick={() => onConfirmDelete(false)}
              className="rounded-md px-2 py-1 text-[11px] font-medium text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            >
              {t("launch.editor.cancel")}
            </button>
          </span>
        ) : (
          <button
            type="button"
            onClick={() => onConfirmDelete(true)}
            aria-label={t("launch.editor.delete")}
            title={t("launch.editor.delete")}
            className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-rose-300"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-1.5 px-2">
        <span className="mr-1 text-[11px] text-sf-muted">{t("launch.editor.aircraft")}</span>
        {draft.aircraft.map((a) => (
          <span
            key={a}
            className="inline-flex items-center gap-1 rounded-md bg-sf-raised py-0.5 pl-1.5 pr-0.5 font-mono text-[11px] text-sf-text"
          >
            {a}
            <button
              type="button"
              onClick={() => onChange((d) => ({ ...d, aircraft: d.aircraft.filter((x) => x !== a) }))}
              aria-label={t("launch.editor.remove_type", { type: a })}
              className="rounded p-0.5 text-sf-muted hover:text-sf-text"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <input
          list="launch-aircraft-types"
          value={typed}
          onChange={(e) => {
            const v = e.target.value;
            // Elegido de la lista o separado por coma: se añade.
            if (/[,;\s]$/.test(v) || suggestions.includes(v.trim().toUpperCase())) addTypes(v);
            else setTyped(v);
          }}
          onBlur={() => addTypes(typed)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addTypes(typed);
            }
          }}
          placeholder={t("launch.editor.aircraft_placeholder")}
          title={t("launch.editor.aircraft_hint")}
          className="w-28 rounded-md border border-sf-border bg-sf-bg px-1.5 py-0.5 font-mono text-[11px] uppercase text-sf-text outline-none placeholder:normal-case placeholder:text-sf-muted/60 focus:border-brand-500/60"
        />
        <datalist id="launch-aircraft-types">
          {suggestions.map((a) => (
            <option key={a} value={a} />
          ))}
        </datalist>
      </div>
    </div>
  );
}

function StepRow({
  step,
  first,
  missing,
  simLabel,
  simMissing,
  expanded,
  onToggle,
  onChange,
  onRemove,
}: {
  step: LaunchStep;
  first: boolean;
  missing: boolean;
  simLabel: string;
  simMissing: boolean;
  expanded: boolean;
  onToggle: () => void;
  onChange: (patch: Partial<LaunchStep>) => void;
  onRemove: () => void;
}) {
  const controls = useDragControls();
  const isApp = step.kind === "app";
  const isSim = step.kind === "sim";
  return (
    <Reorder.Item as="li" value={step} dragListener={false} dragControls={controls} className="list-none">
      <WaitChip step={step} first={first} onChange={onChange} />
      <div className="rounded-xl border border-sf-border bg-sf-surface">
        <div className="grid grid-cols-[auto_auto_minmax(0,1fr)_8.5rem_10.5rem_auto] items-center gap-2 px-2.5 py-2">
          <button
            type="button"
            onPointerDown={(e) => controls.start(e)}
            aria-label={t("launch.editor.drag")}
            title={t("launch.editor.drag")}
            className="cursor-grab touch-none text-sf-muted/60 hover:text-sf-text active:cursor-grabbing"
          >
            <GripVertical className="h-4 w-4" />
          </button>
          <span className="flex h-6 w-6 items-center justify-center">{KIND_ICON[step.kind]}</span>
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-xs font-medium text-sf-text">{isSim ? simLabel : step.name}</span>
            {(missing || (isSim && simMissing)) && (
              <span
                className="inline-flex shrink-0 items-center gap-1 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-300"
                title={step.target}
              >
                <TriangleAlert className="h-3 w-3" />
                {t("launch.editor.missing")}
              </span>
            )}
            {isApp && step.open === "admin" && (
              <span title={t("launch.editor.admin_hint")}>
                <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-amber-300" />
              </span>
            )}
          </span>
          {isApp ? (
            <select
              value={step.open}
              onChange={(e) => onChange({ open: e.target.value as LaunchOpen })}
              aria-label={t("launch.editor.open_label")}
              className={SELECT}
            >
              {OPEN_MODES.map((m) => (
                <option key={m} value={m}>
                  {t(`launch.open.${m}`)}
                </option>
              ))}
            </select>
          ) : (
            <span className={PILL}>{openLabel(step)}</span>
          )}
          {isApp ? (
            <select
              value={step.close}
              onChange={(e) => onChange({ close: e.target.value as LaunchClose })}
              aria-label={t("launch.editor.close_label")}
              className={SELECT}
            >
              {CLOSE_MODES.map((m) => (
                <option key={m} value={m}>
                  {t(`launch.close.${m}`)}
                </option>
              ))}
            </select>
          ) : (
            <span className={PILL}>{isSim ? "—" : t("launch.close.stay")}</span>
          )}
          <span className="flex items-center">
            <button
              type="button"
              onClick={() => void useLaunchStore.getState().testStep(step)}
              aria-label={t("launch.editor.test")}
              title={t("launch.editor.test")}
              disabled={isSim}
              className={cn("rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-emerald-300", isSim && "invisible")}
            >
              <Play className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onClick={onToggle}
              aria-label={t("launch.editor.details")}
              title={t("launch.editor.details")}
              disabled={isSim}
              className={cn("rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text", isSim && "invisible")}
            >
              <ChevronDown className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")} />
            </button>
            <button
              type="button"
              onClick={onRemove}
              aria-label={t("launch.editor.remove_step")}
              title={t("launch.editor.remove_step")}
              className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-rose-300"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </span>
        </div>
        {expanded && !isSim && <StepDetails step={step} onChange={onChange} />}
      </div>
    </Reorder.Item>
  );
}

/** La espera antes del paso: un chip que despliega las opciones debajo. */
function WaitChip({
  step,
  first,
  onChange,
}: {
  step: LaunchStep;
  first: boolean;
  onChange: (patch: Partial<LaunchStep>) => void;
}) {
  const [open, setOpen] = useState(false);
  const moment = step.wait !== "seconds";
  const opt = (active: boolean) =>
    cn(
      "rounded-lg px-2 py-1 text-[11px] font-medium",
      active ? "bg-brand-500/15 text-brand-300 ring-1 ring-brand-500/30" : "text-sf-muted hover:bg-sf-raised hover:text-sf-text",
    );
  return (
    <div className="flex flex-col items-center gap-1 py-1">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title={t("launch.editor.wait_hint")}
        className={cn(
          "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px]",
          moment ? "bg-sky-500/10 text-sky-300" : "bg-sf-raised text-sf-muted hover:text-sf-text",
        )}
      >
        <Clock className="h-3 w-3" />
        {waitLabel(step, first)}
      </button>
      {open && (
        <div className="flex flex-wrap items-center justify-center gap-1 rounded-xl border border-sf-border bg-sf-surface p-1">
          <button
            type="button"
            className={opt(!moment && step.waitSeconds === 0)}
            onClick={() => onChange({ wait: "seconds", waitSeconds: 0 })}
          >
            {first ? t("launch.wait.start") : t("launch.wait.now")}
          </button>
          <span className={cn(opt(!moment && step.waitSeconds > 0), "inline-flex items-center gap-1")}>
            {t("launch.wait.custom")}
            <input
              type="number"
              min={1}
              max={600}
              step={1}
              value={!moment && step.waitSeconds > 0 ? step.waitSeconds : ""}
              placeholder="5"
              onChange={(e) =>
                onChange({
                  wait: "seconds",
                  waitSeconds: Math.min(600, Math.max(0, Math.round(Number(e.target.value) || 0))),
                })
              }
              aria-label={t("launch.wait.custom")}
              className="w-12 rounded border border-sf-border bg-sf-bg px-1 py-0.5 text-center text-[11px] text-sf-text outline-none focus:border-brand-500/60"
            />
            {t("launch.wait.unit")}
          </span>
          <button
            type="button"
            className={opt(step.wait === "menu")}
            onClick={() => onChange({ wait: "menu", waitSeconds: 0 })}
          >
            {t("launch.wait.menu")}
          </button>
          <button
            type="button"
            className={opt(step.wait === "loaded")}
            onClick={() => onChange({ wait: "loaded", waitSeconds: 0 })}
          >
            {t("launch.wait.loaded")}
          </button>
        </div>
      )}
    </div>
  );
}

function StepDetails({ step, onChange }: { step: LaunchStep; onChange: (patch: Partial<LaunchStep>) => void }) {
  const [urlError, setUrlError] = useState(false);
  const pick = async () => {
    if (step.kind === "app") {
      const path = await api.pickFilePath([{ name: t("launch.editor.pick_exe"), extensions: ["exe"] }]);
      if (path) onChange({ target: path, process: [] });
    } else if (step.kind === "folder") {
      const path = await api.pickFolderPath();
      if (path) onChange({ target: path });
    } else if (step.kind === "script") {
      const path = await api.pickFilePath([{ name: t("launch.editor.pick_script"), extensions: SCRIPT_EXTENSIONS }]);
      if (path) onChange({ target: path });
    }
  };
  return (
    <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-2 border-t border-sf-border px-3 py-2.5 text-[11px] text-sf-muted">
      <span>{t("launch.editor.field_name")}</span>
      <CommitInput
        value={step.name}
        maxLength={80}
        onCommit={(name) => onChange({ name })}
        className={FIELD}
      />
      <span>{t(step.kind === "web" ? "launch.editor.field_url" : "launch.editor.field_path")}</span>
      {step.kind === "web" ? (
        <div>
          <CommitInput
            value={step.target}
            onCommit={(target) => {
              const ok = isWebUrl(target);
              setUrlError(!ok);
              if (ok) onChange({ target: target.trim() });
            }}
            className={FIELD}
          />
          {urlError && <p className="mt-1 text-rose-300">{t("launch.editor.web_invalid")}</p>}
        </div>
      ) : (
        <div className="flex min-w-0 items-center gap-2">
          <span className={cn(FIELD, "truncate bg-sf-raised text-sf-muted")} title={step.target}>
            {step.target}
          </span>
          <button
            type="button"
            onClick={() => void pick()}
            className="shrink-0 rounded-lg border border-sf-border px-2.5 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("launch.editor.change")}
          </button>
        </div>
      )}
      {step.kind === "script" && (
        <>
          <span>{t("launch.editor.field_args")}</span>
          <CommitInput value={step.args} onCommit={(args) => onChange({ args })} className={FIELD} />
          <span />
          <p className="text-sf-muted">{t("launch.editor.script_hint")}</p>
        </>
      )}
      {step.kind === "app" && (
        <>
          <span>{t("launch.editor.field_args")}</span>
          <CommitInput value={step.args} onCommit={(args) => onChange({ args })} className={FIELD} />
          <span title={t("launch.editor.field_process_hint")}>{t("launch.editor.field_process")}</span>
          <CommitInput
            value={step.process.join(", ")}
            placeholder={step.target.split(/[\\/]/).pop()}
            title={t("launch.editor.field_process_hint")}
            onCommit={(v) =>
              onChange({
                process: v
                  .split(",")
                  .map((p) => p.trim())
                  .filter(Boolean),
              })
            }
            className={FIELD}
          />
        </>
      )}
    </div>
  );
}

function AddBar({
  draft,
  apps,
  simAvailable,
  onAdd,
}: {
  draft: LaunchSequence;
  apps: ReturnType<typeof useLaunchStore.getState>["apps"];
  simAvailable: boolean;
  onAdd: (step: LaunchStep) => void;
}) {
  const [webOpen, setWebOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [urlError, setUrlError] = useState(false);
  const free = apps.filter((a) => !draft.steps.some((s) => s.target.toLowerCase() === a.target.toLowerCase()));
  const hasSim = draft.steps.some((s) => s.kind === "sim");

  const addFile = async () => {
    const path = await api.pickFilePath([{ name: t("launch.editor.pick_exe"), extensions: ["exe"] }]);
    if (path) onAdd(appStep(path));
  };
  const addFolder = async () => {
    const path = await api.pickFolderPath();
    if (path) onAdd(folderStep(path));
  };
  const addScript = async () => {
    const path = await api.pickFilePath([{ name: t("launch.editor.pick_script"), extensions: SCRIPT_EXTENSIONS }]);
    if (path) onAdd(scriptStep(path));
  };
  const addWeb = () => {
    const u = url.trim();
    if (!isWebUrl(u)) {
      setUrlError(true);
      return;
    }
    onAdd(webStep(u));
    setUrl("");
    setUrlError(false);
    setWebOpen(false);
  };

  const add = "inline-flex items-center gap-1.5 rounded-lg border border-sf-border px-2.5 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised";
  return (
    <div className="space-y-2 border-t border-sf-border px-5 py-3">
      {free.length > 0 && (
        <div className="flex items-start gap-2">
          <span className="shrink-0 pt-1 text-[11px] text-sf-muted">{t("launch.editor.found")}</span>
          <div className="flex max-h-[3.5rem] min-w-0 flex-wrap gap-1.5 overflow-y-auto">
            {free.map((a) => (
              <button
                key={a.key}
                type="button"
                onClick={() => onAdd(stepFromApp(a))}
                title={a.target}
                className="inline-flex items-center gap-1 rounded-md bg-sf-raised px-2 py-1 text-[11px] text-sf-text hover:ring-1 hover:ring-brand-500/40"
              >
                <Plus className="h-3 w-3 text-sf-muted" />
                {a.name}
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={() => void addFile()} className={add}>
          <AppWindow className="h-3.5 w-3.5 text-sf-muted" />
          {t("launch.editor.add_app")}
        </button>
        <button type="button" onClick={() => setWebOpen((o) => !o)} className={add}>
          <Globe className="h-3.5 w-3.5 text-sky-400" />
          {t("launch.editor.add_web")}
        </button>
        <button type="button" onClick={() => void addFolder()} className={add}>
          <FolderOpen className="h-3.5 w-3.5 text-amber-300" />
          {t("launch.editor.add_folder")}
        </button>
        <button type="button" onClick={() => void addScript()} className={add}>
          <TerminalSquare className="h-3.5 w-3.5 text-violet-300" />
          {t("launch.editor.add_script")}
        </button>
        {!hasSim && simAvailable && (
          <button type="button" onClick={() => onAdd(simStep())} className={add}>
            <Plane className="h-3.5 w-3.5 text-brand-400" />
            {t("launch.editor.add_sim")}
          </button>
        )}
        {webOpen && (
          <span className="flex min-w-[16rem] flex-1 items-center gap-2">
            <input
              autoFocus
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                setUrlError(false);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") addWeb();
              }}
              placeholder={t("launch.editor.web_placeholder")}
              className={cn(FIELD, urlError && "border-rose-500/60")}
            />
            <button type="button" onClick={addWeb} className={add}>
              {t("launch.editor.web_add")}
            </button>
          </span>
        )}
      </div>
      {urlError && <p className="text-[11px] text-rose-300">{t("launch.editor.web_invalid")}</p>}
    </div>
  );
}

/** «Vista previa»: el orden y los tiempos, sin abrir nada. */
function Preview({ draft, simLabel }: { draft: LaunchSequence; simLabel: string }) {
  const closes = draft.steps.filter((s) => s.close === "exit").map((s) => s.name);
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      <p className="mb-3 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("launch.preview.title")}</p>
      <ol className="space-y-0">
        {draft.steps.map((s, i) => (
          <li key={s.id} className="grid grid-cols-[minmax(0,13rem)_1.5rem_minmax(0,1fr)] items-stretch gap-2">
            <span className="py-2 text-right text-[11px] text-sf-muted">{waitLabel(s, i === 0)}</span>
            <span className="relative flex justify-center">
              <span className="absolute inset-y-0 w-px bg-sf-border" />
              <span className="relative z-10 mt-2.5 h-2.5 w-2.5 rounded-full bg-brand-400" />
            </span>
            <span className="flex min-w-0 items-center gap-2 py-2">
              {KIND_ICON[s.kind]}
              <span className="truncate text-xs font-medium text-sf-text">{s.kind === "sim" ? simLabel || s.name : s.name}</span>
              <span className="shrink-0 text-[11px] text-sf-muted">· {openLabel(s)}</span>
            </span>
          </li>
        ))}
      </ol>
      {closes.length > 0 && (
        <p className="mt-4 rounded-xl border border-sf-border bg-sf-surface px-3.5 py-2.5 text-[11px] leading-snug text-sf-muted">
          {t("launch.preview.closes", { names: closes.join(", ") })}
        </p>
      )}
    </div>
  );
}
