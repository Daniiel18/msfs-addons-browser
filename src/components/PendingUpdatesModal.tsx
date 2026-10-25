import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { create } from "zustand";
import { ArrowRight, Download, EyeOff, Map as MapIcon, Plane, Sparkles, User, X } from "lucide-react";
import type { AvailableUpdate, CommunityPackage } from "../lib/types";
import { useAppStore } from "../stores/useAppStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useFlightsimUpdateStore, useInstalledFlightsimUpdates } from "../stores/useFlightsimUpdateStore";
import { t } from "../lib/i18n";
import { useFstoEmbedStore } from "./FlightsimEmbed";

/** (25/10) Abre/cierra el modal de updates pendientes (aviso y tarjeta de
 *  Inicio). */
export const usePendingUpdatesModal = create<{
  open: boolean;
  setOpen: (open: boolean) => void;
}>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));

const isIcao = (s: string) => /^[A-Z]{4}$/i.test(s.trim());

/** (25/10) "Ver en el mapa" sólo para escenarios: el paquete instalado es
 *  SCENERY y la update trae su ICAO (un avión o una utilidad no están en el
 *  mapa). Misma regla en la campanita y en el modal. */
export function updateOnMap(u: AvailableUpdate, packages: CommunityPackage[]): boolean {
  if (!isIcao(u.icao)) return false;
  const pkg = packages.find((p) => p.folderName === u.folderName);
  return (pkg?.contentType ?? "").toUpperCase() === "SCENERY";
}

/** (25/10) Lo que pinta una tarjeta, venga del catálogo o de flightsim.to. */
interface CardItem {
  key: string;
  title: string;
  developer: string;
  sourceName: string;
  icao: string;
  imageUrl: string | null;
  from: string;
  to: string;
  openTooltip: string;
  onOpen: () => void;
  onMap?: () => void;
  onIgnore: () => void;
}

/** (25/10) Cuántas updates hay en total (catálogos + flightsim.to): la
 *  cuenta de Inicio y la del modal son la misma. */
export function usePendingUpdatesCount(): number {
  const catalog = useCommunityStore((s) => s.updates.length);
  const fsto = useInstalledFlightsimUpdates().length;
  return catalog + fsto;
}

/**
 * (25/10) Addons con versión nueva en cualquier catálogo (SceneryAddons,
 * Simplaza, aerohub) y en flightsim.to, en tarjetas como las de Explorar: la
 * imagen con su fuente y "Update vX → vY" encima. Clic → su tarjeta en
 * Explorar (catálogos) o su página en la ventana de flightsim.to. Por
 * tarjeta: ignorar esta versión y, en escenarios, verlo en el mapa; abajo,
 * actualizar todas las de catálogo con el asistente.
 */
export function PendingUpdatesModal() {
  const open = usePendingUpdatesModal((s) => s.open);
  const setOpen = usePendingUpdatesModal((s) => s.setOpen);
  const updates = useCommunityStore((s) => s.updates);
  const dismissUpdate = useCommunityStore((s) => s.dismissUpdate);
  const startUpdateAll = useCommunityStore((s) => s.startUpdateAll);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const packages = useCommunityStore((s) => s.packages);
  const sources = useAppStore((s) => s.sources);
  const setView = useAppStore((s) => s.setView);
  const openCatalogAddon = useAppStore((s) => s.openCatalogAddon);
  const openFlightsimTab = useAppStore((s) => s.openFlightsimTab);
  const fstoUpdates = useInstalledFlightsimUpdates();
  const ackFsto = useFlightsimUpdateStore((s) => s.ackUpdate);
  const total = updates.length + fstoUpdates.length;
  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  // Si se ignoran todas, no queda nada que enseñar.
  useEffect(() => {
    if (open && total === 0) setOpen(false);
  }, [open, total, setOpen]);

  const sourceName = (id: string) => sources.find((s) => s.id === id)?.name ?? id;
  const goTo = (u: AvailableUpdate) => {
    close();
    void openCatalogAddon(u);
  };
  const onMap = (u: AvailableUpdate) => {
    close();
    setView("map");
    setFocused(u.folderName);
  };
  const updateAll = () => {
    close();
    startUpdateAll();
  };

  const items: CardItem[] = [
    ...updates.map((u) => ({
      key: `cat:${u.folderName}`,
      title: u.title,
      developer: u.developer,
      sourceName: sourceName(u.source),
      icao: u.icao,
      imageUrl: u.imageUrl,
      from: u.installedVersion,
      to: u.latestVersion,
      openTooltip: t("updates.modal.go_tooltip", { source: sourceName(u.source) }),
      onOpen: () => goTo(u),
      onMap: updateOnMap(u, packages) ? () => onMap(u) : undefined,
      onIgnore: () => void dismissUpdate(u.folderName),
    })),
    // Su página en la pestaña flightsim.to de Explorar, como los demás
    // catálogos. Abrirla para re-descargar avanza la versión de referencia,
    // como ignorarla.
    ...fstoUpdates.map((u) => ({
      key: `fsto:${u.folderName}`,
      title: u.title ?? u.folderName,
      developer: "",
      sourceName: "flightsim.to",
      icao: "",
      imageUrl: u.thumbnail,
      from: u.currentVersion ?? "?",
      to: u.latestVersion ?? "?",
      openTooltip: t("updates.modal.go_fsto_tooltip"),
      onOpen: () => {
        close();
        ackFsto(u.folderName);
        useFstoEmbedStore.getState().openUrl(u.link);
        openFlightsimTab();
      },
      onIgnore: () => ackFsto(u.folderName),
    })),
  ].sort((a, b) => a.title.localeCompare(b.title));

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80 px-4 py-8 backdrop-blur-sm"
          onClick={close}
        >
          <motion.div
            initial={{ opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.98 }}
            transition={{ duration: 0.18 }}
            onClick={(e) => e.stopPropagation()}
            className="flex max-h-full w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-amber-500/40 bg-sf-bg shadow-2xl shadow-black/50"
          >
            <header className="flex items-start justify-between gap-3 border-b border-sf-border px-5 py-4">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-4 w-4 text-amber-300" />
                  <h2 className="text-sm font-semibold text-sf-text">{t("updates.modal.title")}</h2>
                  <span className="rounded-md bg-amber-500/15 px-1.5 py-0.5 text-[11px] font-bold text-amber-300 ring-1 ring-amber-500/30">
                    {total}
                  </span>
                </div>
                <p className="mt-0.5 text-xs text-sf-muted">{t("updates.modal.hint")}</p>
              </div>
              <button
                onClick={close}
                aria-label={t("common.close")}
                className="rounded-lg p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-1 gap-4 overflow-y-auto p-5 sm:grid-cols-2 lg:grid-cols-3">
              {items.map((item) => (
                <UpdateCard key={item.key} item={item} />
              ))}
            </div>

            {updates.length > 0 && (
            <footer className="flex items-center justify-end gap-3 border-t border-sf-border px-5 py-3">
              <button
                onClick={updateAll}
                className="inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-amber-500 px-3 py-1.5 text-xs font-bold text-amber-950 shadow-lg shadow-amber-500/20 transition-colors hover:bg-amber-400"
              >
                <Download className="h-3.5 w-3.5" />
                {t("updates.modal.update_all")}
              </button>
            </footer>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** Tarjeta de una update: misma cabecera que las de Explorar (imagen,
 *  fuente/ICAO a la izquierda, "Update vX → vY" a la derecha). */
function UpdateCard({ item: u }: { item: CardItem }) {
  const { sourceName, onOpen, onMap, onIgnore } = u;
  const [failed, setFailed] = useState(false);
  const showImage = !!u.imageUrl && !failed;
  return (
    <article className="group flex flex-col rounded-xl border border-amber-500/50 bg-sf-surface transition-colors hover:border-amber-400/70">
      <button
        onClick={onOpen}
        title={u.openTooltip}
        className="relative block aspect-video w-full overflow-hidden rounded-t-xl bg-gradient-to-br from-slate-800 via-slate-900 to-slate-950 text-left"
      >
        {showImage ? (
          <img
            src={u.imageUrl ?? undefined}
            alt={u.title}
            loading="lazy"
            decoding="async"
            referrerPolicy="no-referrer"
            onError={() => setFailed(true)}
            className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.03]"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            <div className="flex flex-col items-center gap-2 text-slate-600">
              <Plane className="h-9 w-9" strokeWidth={1.5} />
              {isIcao(u.icao) && (
                <span className="font-mono text-2xl font-bold tracking-widest text-slate-600">{u.icao}</span>
              )}
            </div>
          </div>
        )}
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950/60 via-transparent to-slate-950/40" />
        <div className="absolute inset-x-0 top-0 flex flex-wrap items-start justify-between gap-1.5 p-2.5">
          <div className="flex flex-wrap items-center gap-1.5">
            {isIcao(u.icao) && (
              <span className="inline-flex items-center gap-1 rounded-md bg-black/65 px-2 py-0.5 font-mono text-xs font-semibold text-[#a6eedf] ring-1 ring-[#35cdb2]/40">
                <Plane className="h-3 w-3" />
                {u.icao}
              </span>
            )}
            <span className="rounded-md bg-black/65 px-2 py-0.5 text-[11px] font-medium text-white/90 ring-1 ring-white/15">
              {sourceName}
            </span>
          </div>
          <span className="inline-flex items-center gap-1 rounded-md bg-amber-500 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide text-amber-950 shadow-lg shadow-amber-500/30">
            <Sparkles className="h-3 w-3" />
            {t("result.update.label", { from: u.from, to: u.to })}
          </span>
        </div>
      </button>

      <div className="flex flex-1 flex-col p-4">
        <h3 className="line-clamp-2 text-[15px] font-semibold leading-snug text-sf-text" title={u.title}>
          {u.title}
        </h3>
        {u.developer && (
          <div className="mt-1.5 flex items-center gap-1 text-xs text-sf-muted">
            <User className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{u.developer}</span>
          </div>
        )}
        <footer className="mt-auto flex items-center gap-1.5 pt-4">
          <button
            onClick={onIgnore}
            title={t("updates.modal.ignore")}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1.5 text-xs font-medium text-sf-muted transition-colors hover:bg-rose-500/10 hover:text-rose-200"
          >
            <EyeOff className="h-3.5 w-3.5" />
            {t("updates.modal.ignore_short")}
          </button>
          {onMap && (
            <button
              onClick={onMap}
              title={t("common.view_on_map")}
              aria-label={t("common.view_on_map")}
              className="shrink-0 rounded-lg p-2 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            >
              <MapIcon className="h-4 w-4" />
            </button>
          )}
          <button
            onClick={onOpen}
            className="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 transition-colors hover:bg-brand-400"
          >
            {t("updates.modal.open")}
            <ArrowRight className="h-3.5 w-3.5" />
          </button>
        </footer>
      </div>
    </article>
  );
}
