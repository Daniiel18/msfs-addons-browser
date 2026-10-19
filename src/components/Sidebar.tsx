import { useEffect, useRef, useState, type ComponentType } from "react";
import { createPortal } from "react-dom";
import {
  BookOpen,
  Compass,
  Home,
  MapPinned,
  Package,
  PanelLeftClose,
  PanelLeftOpen,
  Plane,
  Radio,
  Settings,
  Wallet,
  Warehouse,
} from "lucide-react";
import { useAppStore, type View } from "../stores/useAppStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { cn } from "../lib/cn";
import { t } from "../lib/i18n";

/**
 * (Rediseño F1) Menú lateral principal — reemplaza a las pestañas
 * horizontales. Dos estados:
 *   · Expandido (232px): iconos + etiquetas + títulos de grupo.
 *   · Contraído (68px): sólo iconos, con tooltip al pasar el ratón.
 *
 * En ventanas estrechas (< 1200px) el menú arranca contraído y, al
 * expandirlo, se superpone al contenido (no lo empuja) para no dejar la
 * vista con ~700px útiles. La preferencia expandido/contraído de ventanas
 * anchas se recuerda en localStorage.
 *
 * Navega con el mismo store que antes (`useAppStore.view`) y conserva los
 * `data-tour-id` que usa el tour de bienvenida.
 */

const STORAGE_KEY = "simfleet.sidebar.expanded";
const NARROW_BREAKPOINT = 1200;

type NavTarget = { kind: "view"; view: View } | { kind: "settings" };

interface NavItem {
  key: string;
  labelKey: string;
  icon: ComponentType<{ className?: string }>;
  tourId: string;
  target: NavTarget;
}

interface NavGroup {
  id: string;
  labelKey: string;
  items: NavItem[];
}

const GROUPS: NavGroup[] = [
  {
    id: "general",
    labelKey: "nav.group.general",
    items: [
      { key: "dashboard", labelKey: "nav.dashboard", icon: Home, tourId: "nav-dashboard", target: { kind: "view", view: "dashboard" } },
    ],
  },
  {
    id: "addons",
    labelKey: "nav.group.addons",
    items: [
      { key: "search", labelKey: "nav.search", icon: Compass, tourId: "nav-search", target: { kind: "view", view: "search" } },
      { key: "addons", labelKey: "nav.addons", icon: Package, tourId: "nav-addons", target: { kind: "view", view: "addons" } },
      { key: "map", labelKey: "nav.map", icon: MapPinned, tourId: "nav-map", target: { kind: "view", view: "map" } },
    ],
  },
  {
    id: "ops",
    labelKey: "nav.group.flight_ops",
    items: [
      { key: "flightbook", labelKey: "nav.flightbook", icon: BookOpen, tourId: "nav-flightbook", target: { kind: "view", view: "flightbook" } },
      { key: "hangar", labelKey: "nav.hangar", icon: Warehouse, tourId: "nav-hangar", target: { kind: "view", view: "hangar" } },
      { key: "economy", labelKey: "nav.economy", icon: Wallet, tourId: "nav-economy", target: { kind: "view", view: "economy" } },
    ],
  },
  {
    id: "settings",
    labelKey: "nav.group.settings",
    items: [
      // El tour sigue apuntando a `header-settings` (antes era el engranaje
      // de la cabecera); se conserva el id para no romperlo.
      { key: "settings", labelKey: "nav.settings", icon: Settings, tourId: "header-settings", target: { kind: "settings" } },
    ],
  },
];

function readExpandedPref(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== "0";
  } catch {
    return true;
  }
}

function writeExpandedPref(expanded: boolean) {
  try {
    localStorage.setItem(STORAGE_KEY, expanded ? "1" : "0");
  } catch {
    /* ignore */
  }
}

const isNarrow = () =>
  typeof window !== "undefined" && window.innerWidth < NARROW_BREAKPOINT;

interface Tip {
  label: string;
  top: number;
  left: number;
}

interface SidebarProps {
  settingsOpen: boolean;
  onOpenSettings: () => void;
  safeMode: boolean;
}

export function Sidebar({ settingsOpen, onOpenSettings, safeMode }: SidebarProps) {
  const view = useAppStore((s) => s.view);
  const setView = useAppStore((s) => s.setView);
  const openEconomy = useAppStore((s) => s.openEconomy);

  const [pinnedExpanded, setPinnedExpanded] = useState(readExpandedPref);
  const [narrow, setNarrow] = useState(isNarrow);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [tip, setTip] = useState<Tip | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const expanded = narrow ? overlayOpen : pinnedExpanded;
  const overlay = narrow && overlayOpen;

  useEffect(() => {
    const onResize = () => setNarrow(isNarrow());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (!narrow) setOverlayOpen(false);
  }, [narrow]);

  useEffect(() => {
    setTip(null);
  }, [expanded]);

  // Superpuesto: se cierra al hacer clic fuera o con Escape.
  useEffect(() => {
    if (!overlayOpen) return;
    const onDown = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setOverlayOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOverlayOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [overlayOpen]);

  const toggle = () => {
    setTip(null);
    if (narrow) {
      setOverlayOpen((o) => !o);
      return;
    }
    setPinnedExpanded((prev) => {
      const next = !prev;
      writeExpandedPref(next);
      return next;
    });
  };

  const activate = (item: NavItem) => {
    setTip(null);
    if (item.target.kind === "settings") onOpenSettings();
    else if (item.target.view === "economy") openEconomy(null);
    else setView(item.target.view);
    if (narrow) setOverlayOpen(false);
  };

  const showTip = (el: HTMLElement, label: string) => {
    if (expanded) return;
    const r = el.getBoundingClientRect();
    setTip({ label, top: r.top + r.height / 2, left: r.right + 10 });
  };
  const hideTip = () => setTip(null);

  const isActive = (item: NavItem) =>
    item.target.kind === "settings" ? settingsOpen : view === item.target.view;

  const toggleLabel = expanded ? t("sidebar.collapse") : t("sidebar.expand");

  return (
    <aside
      aria-label={t("sidebar.aria")}
      className={cn(
        // z-[45]: por encima de la barra superior (z-40) cuando el menú se
        // superpone en ventanas estrechas; los modales (z-50+) quedan encima.
        "relative z-[45] h-full shrink-0 transition-[width] duration-200 ease-out",
        narrow || !pinnedExpanded ? "w-sidebar-collapsed" : "w-sidebar-expanded",
      )}
    >
      <div
        ref={panelRef}
        className={cn(
          "flex h-full flex-col overflow-hidden border-r border-sf-border/70 bg-sf-surface transition-[width] duration-200 ease-out",
          expanded ? "w-sidebar-expanded" : "w-sidebar-collapsed",
          overlay && "absolute inset-y-0 left-0 shadow-2xl shadow-black/40",
        )}
      >
        {/* Marca — también es zona de arrastre de la ventana (Tauri). */}
        <div
          data-tauri-drag-region
          className={cn(
            "flex h-11 shrink-0 select-none items-center gap-2.5 border-b border-sf-border/70",
            expanded ? "px-4" : "justify-center",
          )}
        >
          <div
            data-tauri-drag-region
            className="pointer-events-none relative flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-brand-500/15 ring-1 ring-brand-500/35"
          >
            <Plane className="h-4 w-4 text-brand-300" />
            {!expanded && safeMode && (
              <span className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full bg-rose-400 ring-2 ring-sf-surface" />
            )}
          </div>
          {expanded && (
            <span className="pointer-events-none truncate text-sm font-semibold tracking-wide text-sf-text">
              SimFleet
            </span>
          )}
          {expanded && safeMode && (
            <span className="pointer-events-none rounded bg-rose-500/20 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider text-rose-300 ring-1 ring-rose-500/40">
              {t("safeMode.badge")}
            </span>
          )}
        </div>

        <nav className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-3 py-3">
          {GROUPS.map((group, gi) => (
            <div key={group.id} className={gi > 0 ? "mt-4" : undefined}>
              {expanded ? (
                <div className="mb-1.5 truncate px-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-sf-muted/80">
                  {t(group.labelKey)}
                </div>
              ) : gi > 0 ? (
                <div className="mx-2 mb-3 h-px bg-sf-border/70" />
              ) : null}
              <ul className="space-y-1">
                {group.items.map((item) => {
                  const label = t(item.labelKey);
                  return (
                    <li key={item.key}>
                      <SidebarButton
                        icon={item.icon}
                        label={label}
                        expanded={expanded}
                        active={isActive(item)}
                        tourId={item.tourId}
                        onClick={() => activate(item)}
                        onShowTip={(el) => showTip(el, label)}
                        onHideTip={hideTip}
                      />
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </nav>

        <div className="shrink-0 space-y-1 border-t border-sf-border/70 px-3 py-3">
          <SimStatus
            expanded={expanded}
            onShowTip={showTip}
            onHideTip={hideTip}
          />
          <SidebarButton
            icon={expanded ? PanelLeftClose : PanelLeftOpen}
            label={toggleLabel}
            expanded={expanded}
            active={false}
            ariaExpanded={expanded}
            onClick={toggle}
            onShowTip={(el) => showTip(el, toggleLabel)}
            onHideTip={hideTip}
          />
        </div>
      </div>

      {tip &&
        !expanded &&
        createPortal(
          <div
            role="tooltip"
            style={{ top: tip.top, left: tip.left }}
            className="pointer-events-none fixed z-[90] -translate-y-1/2 whitespace-nowrap rounded-md border border-sf-border bg-sf-raised px-2.5 py-1 text-xs font-medium text-sf-text shadow-lg shadow-black/30"
          >
            {tip.label}
          </div>,
          document.body,
        )}
    </aside>
  );
}

function SidebarButton({
  icon: Icon,
  label,
  expanded,
  active,
  tourId,
  ariaExpanded,
  onClick,
  onShowTip,
  onHideTip,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  expanded: boolean;
  active: boolean;
  tourId?: string;
  ariaExpanded?: boolean;
  onClick: () => void;
  onShowTip: (el: HTMLElement) => void;
  onHideTip: () => void;
}) {
  return (
    <button
      type="button"
      data-tour-id={tourId}
      onClick={onClick}
      onMouseEnter={(e) => onShowTip(e.currentTarget)}
      onMouseLeave={onHideTip}
      onFocus={(e) => onShowTip(e.currentTarget)}
      onBlur={onHideTip}
      aria-label={expanded ? undefined : label}
      aria-current={active ? "page" : undefined}
      aria-expanded={ariaExpanded}
      className={cn(
        "relative flex h-10 w-full items-center rounded-lg text-sm font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-brand-500/60",
        expanded ? "gap-3 px-3" : "justify-center",
        active
          ? "bg-brand-500/15 text-brand-200"
          : "text-sf-muted hover:bg-sf-raised hover:text-sf-text",
      )}
    >
      {active && (
        <span
          aria-hidden
          className="absolute -left-3 bottom-2 top-2 w-[3px] rounded-r-full bg-brand-500"
        />
      )}
      <Icon className={cn("h-[18px] w-[18px] shrink-0", active && "text-brand-300")} />
      {expanded && <span className="truncate">{label}</span>}
    </button>
  );
}

/** Estado del simulador al pie del menú (versión elegida + conexión). */
function SimStatus({
  expanded,
  onShowTip,
  onHideTip,
}: {
  expanded: boolean;
  onShowTip: (el: HTMLElement, label: string) => void;
  onHideTip: () => void;
}) {
  const status = useFlightLogStore((s) => s.status);
  const simVersion = useSettingsStore((s) => s.settings.simVersion);

  const simLabel =
    simVersion === "msfs2020"
      ? "MSFS 2020"
      : simVersion === "msfs2024"
        ? "MSFS 2024"
        : "MSFS";
  const state = status?.simconnectConnected
    ? "connected"
    : status?.simRunning
      ? "running"
      : "offline";
  const stateLabel = t(`sidebar.sim.${state}`);
  const dot =
    state === "connected"
      ? "bg-emerald-400"
      : state === "running"
        ? "bg-sf-warning"
        : "bg-slate-500";

  return (
    <div
      onMouseEnter={(e) => onShowTip(e.currentTarget, `${simLabel} · ${stateLabel}`)}
      onMouseLeave={onHideTip}
      className={cn(
        "flex items-center rounded-lg py-2",
        expanded ? "gap-3 px-3" : "justify-center",
      )}
    >
      <div className="relative flex h-[18px] w-[18px] shrink-0 items-center justify-center text-sf-muted">
        <Radio className="h-[18px] w-[18px]" />
        <span
          className={cn(
            "absolute -bottom-0.5 -right-0.5 h-2 w-2 rounded-full ring-2 ring-sf-surface",
            dot,
          )}
        />
      </div>
      {expanded && (
        <div className="min-w-0 leading-tight">
          <div className="truncate text-xs font-semibold text-sf-text">{simLabel}</div>
          <div className="truncate text-[11px] text-sf-muted">{stateLabel}</div>
        </div>
      )}
    </div>
  );
}
