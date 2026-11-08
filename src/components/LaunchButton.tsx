import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, Layers, Loader2, Rocket, Settings2 } from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import type { DispatchFlight } from "../lib/dispatch";
import { pickSequence } from "../lib/launchFlow";
import type { DispatchMode } from "../lib/types";
import { useDispatchPlan, useDispatchStore } from "../stores/useDispatchStore";
import { useLaunchStore } from "../stores/useLaunchStore";
import { DISPATCH_MODES, dispatchLine } from "./DispatchSelector";
import { ActiveSetBadge } from "./ActiveSetBadge";

interface Props {
  /** Tipo ICAO del avión del OFP (elige la secuencia). */
  aircraftIcao: string | null | undefined;
  /** Vuelo para los modos de lanzamiento; sin él, sólo los conjuntos. */
  flight?: DispatchFlight | null;
  size?: "md" | "sm";
  className?: string;
}

/**
 * (02/11) Etapa 6 — botón «Lanzar» (En vuelo y pre-vuelo): abre la secuencia
 * de arranque del avión del OFP; la flecha deja elegir otra o editarlas.
 */
export function LaunchButton({ aircraftIcao, flight = null, size = "md", className }: Props) {
  const sequences = useLaunchStore((s) => s.sequences);
  const loaded = useLaunchStore((s) => s.loaded);
  const run = useLaunchStore((s) => s.run);
  const chosen = useLaunchStore((s) => s.chosen);
  const mode = useDispatchStore((s) => s.mode);
  const setId = useDispatchStore((s) => s.setId);
  const sets = useDispatchStore((s) => s.sets);
  const useAlternate = useDispatchStore((s) => s.useAlternate);
  const trip = useDispatchStore((s) => s.trip);
  const plan = useDispatchPlan(flight);
  // En portal: el panel de En vuelo y el modal del pre-vuelo recortan lo que
  // sobresale. `null` = cerrado.
  const [menuPos, setMenuPos] = useState<CSSProperties | null>(null);
  const open = menuPos !== null;
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Con un viaje activo, la secuencia sigue al avión del viaje.
  const type = (trip?.aircraft ?? aircraftIcao)?.trim().toUpperCase() ?? "";
  const auto = useMemo(() => pickSequence(sequences, type), [sequences, type]);
  const manual = chosen && chosen.aircraft === type ? sequences.find((s) => s.id === chosen.id) : undefined;
  const seq = manual ?? auto.seq;
  const running = run?.state === "running";
  const done = run ? run.steps.filter((s) => !["pending", "waiting", "opening"].includes(s.status)).length : 0;

  useEffect(() => {
    if (!loaded) void useLaunchStore.getState().load();
  }, [loaded]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (!ref.current?.contains(target) && !menuRef.current?.contains(target)) setMenuPos(null);
    };
    // En captura: que Escape cierre el menú y no el modal de debajo.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setMenuPos(null);
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const toggleMenu = () => {
    const box = ref.current?.getBoundingClientRect();
    if (open || !box) {
      setMenuPos(null);
      return;
    }
    const right = window.innerWidth - box.right;
    // Hacia donde haya sitio.
    const below = window.innerHeight - box.bottom;
    setMenuPos(below >= box.top ? { top: box.bottom + 6, right } : { bottom: window.innerHeight - box.top + 6, right });
  };

  const go = async () => {
    if (!seq) {
      useLaunchStore.getState().openEditor();
      return;
    }
    if (running || busy) return;
    // (03/11) Etapa 7: sin confirmar no se apaga nada, y con varios aviones
    // posibles se pregunta cuál.
    if (plan.needsReview) {
      useDispatchStore.getState().openReview({ kind: "flight", flight: plan.flight, seqId: seq.id });
      return;
    }
    setBusy(true);
    await useLaunchStore.getState().launch(seq.id, false, plan.changes ?? undefined);
    setBusy(false);
  };

  const pickMode = (m: DispatchMode, id: number | null = null) => {
    void useDispatchStore.getState().setMode(m, id);
  };
  const line = dispatchLine(plan);

  const sm = size === "sm";
  const label = running
    ? t("launch.btn.running", { done: String(done), total: String(run?.steps.length ?? 0) })
    : seq
      ? t("launch.btn.launch_named", { name: seq.name })
      : t("launch.btn.create");

  return (
    <div ref={ref} className={cn("relative flex", className)}>
      <button
        type="button"
        onClick={() => void go()}
        title={seq ? t("launch.btn.hint", { name: seq.name }) : undefined}
        className={cn(
          "inline-flex min-w-0 flex-1 items-center justify-center gap-2 bg-brand-500 font-semibold text-slate-950 hover:bg-brand-400",
          sm ? "rounded-l-lg px-2.5 py-1.5 text-xs" : "rounded-l-xl px-4 py-2.5 text-sm",
          (running || busy) && "cursor-default hover:bg-brand-500",
        )}
      >
        {running || busy ? (
          <Loader2 className={cn("shrink-0 animate-spin", sm ? "h-3.5 w-3.5" : "h-4 w-4")} />
        ) : (
          <Rocket className={cn("shrink-0", sm ? "h-3.5 w-3.5" : "h-4 w-4")} />
        )}
        <span className={cn("truncate", sm && "max-w-[10rem]")}>{label}</span>
      </button>
      <button
        type="button"
        onClick={toggleMenu}
        aria-label={t("launch.btn.choose")}
        title={t("launch.btn.choose")}
        className={cn(
          "border-l border-slate-950/20 bg-brand-500 text-slate-950 hover:bg-brand-400",
          sm ? "rounded-r-lg px-1.5" : "rounded-r-xl px-2.5",
        )}
      >
        <ChevronDown className={sm ? "h-3.5 w-3.5" : "h-4 w-4"} />
      </button>

      {menuPos &&
        createPortal(
        <div
          ref={menuRef}
          style={menuPos}
          className="fixed z-[80] max-h-[calc(100vh-1.5rem)] w-72 overflow-y-auto rounded-xl border border-sf-border bg-sf-surface p-1.5 text-left shadow-2xl"
        >
          <p className="px-2 pb-1.5 pt-1 text-[11px] text-sf-muted">
            {type ? t("launch.menu.for_type", { type }) : t("launch.menu.no_ofp")}
          </p>
          <ul className="max-h-64 overflow-y-auto">
            {sequences.map((s) => (
              <li key={s.id}>
                <button
                  type="button"
                  onClick={() => {
                    useLaunchStore.getState().choose(type, s.id);
                    setMenuPos(null);
                  }}
                  className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-sf-raised"
                >
                  <Check className={cn("mt-0.5 h-3.5 w-3.5 shrink-0", s.id === seq?.id ? "text-brand-400" : "invisible")} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-xs font-medium text-sf-text">{s.name}</span>
                    <span className="block truncate text-[11px] text-sf-muted">
                      {s.aircraft.length > 0
                        ? s.aircraft.join(" · ")
                        : s.isDefault
                          ? t("launch.menu.default")
                          : t("launch.menu.no_aircraft")}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <div className="mt-1 border-t border-sf-border pt-1">
            <p className="px-2 pb-1 pt-1 text-[11px] text-sf-muted">{t("dispatch.menu.addons")}</p>
            <ul>
              {DISPATCH_MODES.map((m) => (
                <ModeRow
                  key={m}
                  checked={plan.mode === m}
                  label={t(`dispatch.mode.${m}`)}
                  hint={t(`dispatch.mode.${m}_hint`)}
                  onClick={() => pickMode(m)}
                />
              ))}
              {sets
                .filter((s) => s.kind === "user")
                .map((s) => (
                  <ModeRow
                    key={s.id}
                    checked={mode === "set" && setId === s.id}
                    label={t("dispatch.menu.use_set", { name: s.name })}
                    hint={t("dispatch.sets.counts", { on: s.enabled.length, total: s.known.length })}
                    onClick={() => pickMode("set", s.id)}
                  />
                ))}
              {plan.flight && plan.flight.alternates.length > 0 && (
                <ModeRow
                  checked={useAlternate}
                  label={t("dispatch.menu.alternate")}
                  hint={t("dispatch.menu.alternate_hint", { icao: plan.flight.alternates.join(", ") })}
                  onClick={() => void useDispatchStore.getState().setUseAlternate(!useAlternate)}
                />
              )}
              <ModeRow
                checked={!!trip}
                label={t("dispatch.menu.trip")}
                hint={trip ? trip.stops.join(" → ") : t("dispatch.menu.trip_hint")}
                onClick={() => {
                  setMenuPos(null);
                  useDispatchStore.getState().openTrip(aircraftIcao ?? null);
                }}
              />
            </ul>
            <p className={cn("px-2 py-1 text-[11px]", line.warn ? "font-medium text-amber-300" : "text-sf-muted")}>
              {line.text}
            </p>
            <ActiveSetBadge className="px-2 pb-1" />
            <button
              type="button"
              onClick={() => {
                setMenuPos(null);
                useDispatchStore.getState().openSets();
              }}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
            >
              <Layers className="h-3.5 w-3.5 text-sf-muted" />
              {t("dispatch.menu.manage")}
            </button>
          </div>
          <div className="mt-1 border-t border-sf-border pt-1">
            <button
              type="button"
              onClick={() => {
                setMenuPos(null);
                useLaunchStore.getState().openEditor(seq?.id ?? null);
              }}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
            >
              <Settings2 className="h-3.5 w-3.5 text-sf-muted" />
              {t("launch.menu.edit")}
            </button>
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

function ModeRow({
  checked,
  label,
  hint,
  onClick,
}: {
  checked: boolean;
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-sf-raised"
      >
        <Check className={cn("mt-0.5 h-3.5 w-3.5 shrink-0", checked ? "text-brand-400" : "invisible")} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs font-medium text-sf-text">{label}</span>
          <span className="block truncate text-[11px] text-sf-muted" title={hint}>
            {hint}
          </span>
        </span>
      </button>
    </li>
  );
}
