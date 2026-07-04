import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  BookOpen,
  CornerDownLeft,
  LayoutDashboard,
  Map as MapIcon,
  Package,
  Plane,
  Rocket,
  Search,
  Settings as SettingsIcon,
  Sparkles,
  Warehouse,
  Wallet,
} from "lucide-react";
import { useAppStore } from "../stores/useAppStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { t } from "../lib/i18n";
import type { CommunityPackage } from "../lib/types";

interface Props {
  /** Abre el modal de Ajustes (estado local de App). */
  onOpenSettings: () => void;
}

type Group = "search" | "nav" | "package";

interface PaletteItem {
  id: string;
  group: Group;
  label: string;
  sublabel?: string;
  icon: React.ReactNode;
  /** Texto extra buscable (folder, icao, creator). */
  haystack: string;
  run: () => void;
}

/** ¿Todos los tokens del query aparecen en el heno? (fuzzy simple). */
function tokensMatch(haystack: string, tokens: string[]): boolean {
  if (tokens.length === 0) return true;
  const hay = haystack.toLowerCase();
  return tokens.every((tok) => hay.includes(tok));
}

/**
 * (v6.2.27 / R4) Paleta de comandos global — se abre con Ctrl/Cmd+K
 * desde cualquier pantalla. Un único cuadro para:
 *   · saltar entre vistas (Dashboard, Mapa, Addons, Hangar…) + Ajustes
 *   · saltar a un paquete instalado (aviones/liveries → pestaña Addons
 *     ya filtrada; aeropuertos → Mapa enfocado en él)
 *   · lanzar una búsqueda en el catálogo con lo tecleado
 *
 * Navegación 100% por teclado (↑ ↓ Enter Esc). Registra su propio
 * listener global; no toca el resto de App salvo el callback de Ajustes.
 */
export function CommandPalette({ onOpenSettings }: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const setView = useAppStore((s) => s.setView);
  const openEconomy = useAppStore((s) => s.openEconomy);
  const jumpToAddon = useAppStore((s) => s.jumpToAddon);
  const triggerSearch = useAppStore((s) => s.triggerSearch);
  const packages = useCommunityStore((s) => s.packages);
  const setFocused = useCommunityStore((s) => s.setFocused);

  // Atajo global de apertura.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setOpen((v) => !v);
      }
    };
    // Apertura también por evento (botón del header, sin acoplar estado).
    const onOpenEvt = () => setOpen(true);
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener("simfleet:palette", onOpenEvt);
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener("simfleet:palette", onOpenEvt);
    };
  }, []);

  // Al abrir: limpiar y enfocar.
  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      // El foco tras el mount del input.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const close = () => setOpen(false);

  const items = useMemo<PaletteItem[]>(() => {
    const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const out: PaletteItem[] = [];

    // 1. Búsqueda en catálogo (sólo con texto).
    const raw = query.trim();
    if (raw.length >= 2) {
      out.push({
        id: "search-catalog",
        group: "search",
        label: t("palette.search_catalog", { q: raw }),
        icon: <Search className="h-4 w-4 text-brand-300" />,
        haystack: raw,
        run: () => {
          close();
          void triggerSearch(raw);
        },
      });
    }

    // 2. Navegación + Ajustes.
    const nav: PaletteItem[] = [
      {
        id: "nav-dashboard",
        group: "nav",
        label: t("nav.dashboard"),
        icon: <LayoutDashboard className="h-4 w-4 text-slate-400" />,
        haystack: "dashboard inicio home",
        run: () => {
          close();
          setView("dashboard");
        },
      },
      {
        id: "nav-preflight",
        group: "nav",
        label: t("preflight.cta.title"),
        sublabel: t("palette.on_dashboard"),
        icon: <Rocket className="h-4 w-4 text-emerald-300" />,
        haystack: "listo volar preflight ready fly checklist",
        run: () => {
          close();
          setView("dashboard");
        },
      },
      {
        id: "nav-search",
        group: "nav",
        label: t("nav.search"),
        icon: <Search className="h-4 w-4 text-slate-400" />,
        haystack: "buscar search catalog catálogo",
        run: () => {
          close();
          setView("search");
        },
      },
      {
        id: "nav-map",
        group: "nav",
        label: t("nav.map"),
        icon: <MapIcon className="h-4 w-4 text-slate-400" />,
        haystack: "mapa map escenarios scenery",
        run: () => {
          close();
          setView("map");
        },
      },
      {
        id: "nav-addons",
        group: "nav",
        label: t("nav.addons"),
        icon: <Package className="h-4 w-4 text-slate-400" />,
        haystack: "addons paquetes aviones liveries",
        run: () => {
          close();
          setView("addons");
        },
      },
      {
        id: "nav-flightbook",
        group: "nav",
        label: t("nav.flightbook"),
        icon: <BookOpen className="h-4 w-4 text-slate-400" />,
        haystack: "flightbook logbook vuelos flights bitácora",
        run: () => {
          close();
          setView("flightbook");
        },
      },
      {
        id: "nav-hangar",
        group: "nav",
        label: t("nav.hangar"),
        icon: <Warehouse className="h-4 w-4 text-slate-400" />,
        haystack: "hangar flota fleet matrícula",
        run: () => {
          close();
          setView("hangar");
        },
      },
      {
        id: "nav-economy",
        group: "nav",
        label: t("nav.economy"),
        icon: <Wallet className="h-4 w-4 text-slate-400" />,
        haystack: "finanzas economy dinero aerolínea",
        run: () => {
          close();
          openEconomy(null);
        },
      },
      {
        id: "nav-settings",
        group: "nav",
        label: t("settings.title"),
        icon: <SettingsIcon className="h-4 w-4 text-slate-400" />,
        haystack: "ajustes settings configuración config opciones",
        run: () => {
          close();
          onOpenSettings();
        },
      },
    ];
    for (const n of nav) {
      if (tokensMatch(`${n.label} ${n.haystack}`, tokens)) out.push(n);
    }

    // 3. Paquetes instalados (aviones/liveries → grid; aeropuertos →
    //    mapa enfocado). Sólo con al menos 2 chars para no volcar todo.
    if (tokens.length > 0 && raw.length >= 2) {
      const isAirport = (p: CommunityPackage) =>
        p.latitude != null && p.longitude != null;
      const matched = packages
        .filter((p) =>
          tokensMatch(
            `${p.title} ${p.folderName} ${p.icao ?? ""} ${p.creator ?? ""}`,
            tokens,
          ),
        )
        .slice(0, 6);
      for (const p of matched) {
        const airport = isAirport(p);
        out.push({
          id: `pkg-${p.folderName}`,
          group: "package",
          label: p.title,
          sublabel: [p.icao, airport ? t("palette.on_map") : t("palette.on_addons")]
            .filter(Boolean)
            .join(" · "),
          icon: airport ? (
            <MapIcon className="h-4 w-4 text-sky-300" />
          ) : (
            <Plane className="h-4 w-4 text-fuchsia-300" />
          ),
          haystack: `${p.folderName} ${p.icao ?? ""} ${p.creator ?? ""}`,
          run: () => {
            close();
            if (airport) {
              setFocused(p.folderName);
              setView("map");
            } else {
              jumpToAddon(p.title);
            }
          },
        });
      }
    }

    return out;
  }, [query, packages, setView, openEconomy, jumpToAddon, triggerSearch, setFocused, onOpenSettings]);

  // Reset del índice activo cuando cambia la lista.
  useEffect(() => {
    setActive(0);
  }, [query]);

  // Mantener el activo visible.
  useEffect(() => {
    const el = listRef.current?.querySelector(`[data-idx="${active}"]`);
    (el as HTMLElement | null)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const onInputKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, items.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      items[active]?.run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };

  if (!open) return null;

  return (
    <AnimatePresence>
      <motion.div
        key="palette-backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.1 }}
        onClick={close}
        className="fixed inset-0 z-[80] flex items-start justify-center bg-slate-950/70 px-4 pt-[12vh] backdrop-blur-sm"
      >
        <motion.div
          key="palette"
          initial={{ opacity: 0, scale: 0.98, y: -8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.98, y: -8 }}
          transition={{ duration: 0.14 }}
          onClick={(e) => e.stopPropagation()}
          className="flex max-h-[70vh] w-[min(620px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-700 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <div className="flex items-center gap-3 border-b border-slate-800 px-4 py-3">
            <Search className="h-4 w-4 shrink-0 text-slate-500" />
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onInputKey}
              placeholder={t("palette.placeholder")}
              className="flex-1 bg-transparent text-sm text-slate-100 placeholder:text-slate-600 focus:outline-none"
            />
            <kbd className="hidden shrink-0 rounded border border-slate-700 bg-slate-900 px-1.5 py-0.5 text-[10px] font-medium text-slate-500 sm:inline">
              Esc
            </kbd>
          </div>

          <ul ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-2">
            {items.length === 0 && (
              <li className="px-3 py-6 text-center text-xs text-slate-500">
                {t("palette.no_results")}
              </li>
            )}
            {items.map((it, idx) => (
              <li key={it.id} data-idx={idx}>
                <button
                  onClick={() => it.run()}
                  onMouseEnter={() => setActive(idx)}
                  className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors ${
                    active === idx ? "bg-slate-800" : "hover:bg-slate-900"
                  }`}
                >
                  <span className="shrink-0">{it.icon}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm text-slate-200">
                      {it.label}
                    </span>
                    {it.sublabel && (
                      <span className="block truncate text-[11px] text-slate-500">
                        {it.sublabel}
                      </span>
                    )}
                  </span>
                  {active === idx && (
                    <CornerDownLeft className="h-3.5 w-3.5 shrink-0 text-slate-500" />
                  )}
                </button>
              </li>
            ))}
          </ul>

          <div className="flex items-center justify-between gap-2 border-t border-slate-800 px-4 py-2 text-[10px] text-slate-600">
            <span className="inline-flex items-center gap-1.5">
              <Sparkles className="h-3 w-3" />
              {t("palette.hint")}
            </span>
            <span className="hidden sm:inline">↑ ↓ · Enter · Esc</span>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
