import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { motion } from "framer-motion";
import {
  Calendar,
  CheckCircle2,
  ChevronDown,
  Download,
  ExternalLink,
  FileText,
  Info,
  Plane,
  Sparkles,
  Tag,
  User,
  X,
} from "lucide-react";
import type { Addon, CommunityPackage, DownloadMethod } from "../lib/types";
import { api } from "../lib/tauri";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { isNewer } from "../lib/version";
import { findInstalled } from "../lib/addonMatch";
import { ChangelogModal } from "./ChangelogModal";
import { t, getActiveLocale } from "../lib/i18n";

/**
 * Estado de instalación de un addon de búsqueda respecto a lo que
 * el usuario ya tiene en Community. Se computa correlando por ICAO
 * (el match más fiable que tenemos) entre el resultado del catálogo
 * y la tabla `community_packages`.
 *
 *   · "no-installed" → no aparece en Community.
 *   · "up-to-date"   → instalado, misma versión o superior.
 *   · "update"       → instalado pero con versión menor que el catálogo.
 *   · "installed-unknown-version" → instalado pero alguna versión
 *     no parsea (manifest sin package_version o resultado sin
 *     versión). Avisamos como "instalado" sin afirmar nada sobre
 *     update.
 */
type InstallState =
  | { kind: "not-installed" }
  | { kind: "up-to-date"; pkg: CommunityPackage }
  | { kind: "update"; pkg: CommunityPackage; latestVersion: string }
  | { kind: "installed-unknown-version"; pkg: CommunityPackage };

/** Convierte una fecha del scraping (ISO-8601 o texto crudo del
 *  `<time>` de WordPress) en algo legible al usuario. Si parsea como
 *  ISO la formateamos en `dd MMM yyyy`; si no, devolvemos el texto
 *  tal cual. */
function formatReleaseDate(value: string): string {
  // Heurística rápida: si tiene una `T` o `Z`, parece ISO.
  const looksIso = /[Tz]|[+-]\d{2}:?\d{2}$/i.test(value);
  if (looksIso) {
    const d = new Date(value);
    if (!isNaN(d.getTime())) {
      return d.toLocaleDateString(
        getActiveLocale() === "es" ? "es-ES" : "en-US",
        {
          day: "2-digit",
          month: "short",
          year: "numeric",
        },
      );
    }
  }
  return value;
}

function deriveInstallState(
  addon: Addon,
  packages: CommunityPackage[],
): InstallState {
  const pkg = findInstalled(addon, packages);
  if (!pkg) return { kind: "not-installed" };

  if (!addon.version || !pkg.packageVersion) {
    return { kind: "installed-unknown-version", pkg };
  }
  if (isNewer(addon.version, pkg.packageVersion)) {
    return { kind: "update", pkg, latestVersion: addon.version };
  }
  return { kind: "up-to-date", pkg };
}

interface Props {
  addon: Addon;
}

export function ResultCard({ addon }: Props) {
  const startDownload = useDownloadsStore((s) => s.start);
  const communityPackages = useCommunityStore((s) => s.packages);
  const gsxInstalledIcaos = useGsxLocalStore((s) => s.installedIcaos);
  // Changelog modal — sólo lo exponemos en Simplaza (lo pidió el
  // usuario tras quitarlo del modal de detalle de paquete instalado).
  // SceneryAddons rara vez incluye changelog útil en sus posts y el
  // botón quedaría siempre vacío.
  const [showChangelog, setShowChangelog] = useState(false);
  const isSimplaza = addon.source === "simplaza";
  // (v8.3) Notas de instalación de la ficha (aerohub).
  const [showNotes, setShowNotes] = useState(false);
  // (v8.3) aerohub trae variantes con nombre (2024/2020, liveries, partes…)
  // → se listan todas; el resto de fuentes sigue mostrando las 3 primeras.
  const menuMethods =
    addon.source === "aerohub" ? addon.downloadMethods : addon.downloadMethods.slice(0, 3);

  // Cruza el catálogo con lo que el usuario tiene en Community.
  // Memoizamos: el array `packages` es referencialmente estable
  // entre renders del store, así sólo recomputa cuando realmente
  // cambia la librería (scan, instalación, desinstalación).
  const installState = useMemo(
    () => deriveInstallState(addon, communityPackages),
    [addon, communityPackages],
  );

  // (v5.0.0) El badge GSX sólo se enciende si ESTA ficha es el producto
  // instalado (mismo dev, vía installState) Y hay perfil GSX local para
  // el ICAO. Así no aparece en las fichas de otros devs del mismo ICAO
  // (p. ej. Orbx/JustSim ENGM cuando sólo tienes el de Aerosoft).
  const hasGsxProfile =
    !!(addon.icao && gsxInstalledIcaos.has(addon.icao.toUpperCase())) &&
    installState.kind !== "not-installed";

  // Resaltamos el card entero si hay update — útil cuando la
  // página tiene muchos resultados de versiones distintas y el
  // usuario está buscando "algo nuevo que sustituya lo que ya
  // tengo". Borde ámbar = update; borde verde sutil = al día;
  // borde neutro = no instalado.
  const cardBorder =
    installState.kind === "update"
      ? "border-amber-500/50 hover:border-amber-400/70"
      : installState.kind === "up-to-date" || installState.kind === "installed-unknown-version"
        ? "border-emerald-500/30 hover:border-emerald-400/50"
        : "border-sf-border/80 hover:border-brand-500/40";

  const handleDownload = (method: DownloadMethod) => {
    startDownload({
      addonId: addon.id,
      addonTitle: addon.name,
      source: addon.source,
      method,
      addonSimulator: addon.simulator,
      // (v7.1) Todos los métodos → si el torrent falla, reofrecemos mirrors.
      allMethods: addon.downloadMethods,
      // (v8.3) Se muestran otra vez al guardar un instalador.
      notes: addon.notes ?? null,
    });
  };

  return (
    <motion.article
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18 }}
      // (Rediseño F3) Tarjeta vertical para la rejilla. Sin overflow-hidden
      // aquí (lo lleva el banner) para que el menú de descarga pueda salir.
      className={`group flex flex-col rounded-xl border bg-sf-surface transition-colors ${cardBorder}`}
    >
      {/* Banner superior — ICAO + simulator como overlays sobre la
          imagen. (v1.1.4) Mini-check si el usuario tiene perfil GSX
          local instalado para este ICAO. */}
      <Banner
        imageUrl={addon.imageUrl}
        alt={addon.name}
        icao={addon.icao}
        simulator={addon.simulator}
        installState={installState}
        hasGsxProfile={hasGsxProfile}
      />

      <div className="flex flex-1 flex-col p-4">
        <h3
          className="line-clamp-2 text-[15px] font-semibold leading-snug text-sf-text"
          title={addon.name}
        >
          {addon.name}
        </h3>
        <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-sf-muted">
          {addon.developer && (
            <span className="inline-flex min-w-0 items-center gap-1">
              <User className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{addon.developer}</span>
            </span>
          )}
          {addon.version && (
            <span className="inline-flex items-center gap-1">
              <Tag className="h-3.5 w-3.5" />
              v{addon.version}
            </span>
          )}
          {addon.releasedAt && (
            <span
              className="inline-flex items-center gap-1"
              title={t("result.uploaded_to_source", { date: addon.releasedAt })}
            >
              <Calendar className="h-3.5 w-3.5" />
              {formatReleaseDate(addon.releasedAt)}
            </span>
          )}
        </div>

        <footer className="mt-auto flex items-center gap-1.5 pt-4">
          <button
            onClick={() => api.openExternal(addon.pageUrl)}
            className="shrink-0 rounded-lg p-2 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            title={t("result.open_source")}
            aria-label={t("result.open_source")}
          >
            <ExternalLink className="h-4 w-4" />
          </button>
          {isSimplaza && (
            <button
              onClick={() => setShowChangelog(true)}
              title={t("result.view_simplaza_changelog")}
              className="inline-flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1.5 text-xs font-medium text-sf-muted transition-colors hover:bg-sf-raised hover:text-sf-text"
            >
              <FileText className="h-3.5 w-3.5" />
              {t("result.changelog")}
            </button>
          )}
          {addon.notes && (
            <button
              onClick={() => setShowNotes(true)}
              title={t("result.install_notes")}
              className="inline-flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1.5 text-xs font-medium text-sf-muted transition-colors hover:bg-sf-raised hover:text-sf-text"
            >
              <Info className="h-3.5 w-3.5" />
              {t("result.notes")}
            </button>
          )}
          {addon.downloadMethods.length === 0 ? (
            <span className="ml-auto text-xs text-sf-muted">
              {t("result.no_download_methods")}
            </span>
          ) : (
            <DownloadMenu methods={menuMethods} onPick={handleDownload} />
          )}
        </footer>
      </div>

      {showNotes && addon.notes && (
        <InstallNotesModal
          title={addon.name}
          notes={addon.notes}
          onClose={() => setShowNotes(false)}
        />
      )}

      {showChangelog && (
        <ChangelogModal
          pageUrl={addon.pageUrl}
          title={addon.name}
          sourceLabel="Simplaza"
          version={addon.version}
          onClose={() => setShowChangelog(false)}
        />
      )}
    </motion.article>
  );
}

/** (v8.3) Notas de instalación de la ficha del addon (aerohub). */
function InstallNotesModal({
  title,
  notes,
  onClose,
}: {
  title: string;
  notes: string;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Portal a <body>: la tarjeta es un motion.article (transform) y un
  // `fixed` dentro quedaría atado a ella.
  return createPortal(
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-full w-full max-w-lg flex-col rounded-2xl border border-sf-border bg-sf-bg shadow-2xl"
      >
        <header className="flex items-start justify-between gap-3 border-b border-sf-border px-5 py-4">
          <div className="min-w-0">
            <h2 className="flex items-center gap-2 text-sm font-semibold text-sf-text">
              <Info className="h-4 w-4 text-brand-300" />
              {t("result.install_notes")}
            </h2>
            <p className="mt-0.5 truncate text-xs text-sf-muted">{title}</p>
          </div>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="rounded-lg p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <p className="select-text overflow-y-auto whitespace-pre-line break-words px-5 py-4 text-[13px] leading-relaxed text-sf-text">
          {notes}
        </p>
      </div>
    </div>,
    document.body,
  );
}

/**
 * (Rediseño F3) Botón "Descargar" con el método principal (torrent si lo
 * hay, si no el primero) + flecha que despliega todos los métodos. Cada
 * opción llama al mismo `start` de descargas que antes los botones sueltos.
 */
function DownloadMenu({
  methods,
  onPick,
}: {
  methods: DownloadMethod[];
  onPick: (m: DownloadMethod) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  const primary = methods.find((m) => m.kind === "torrent") ?? methods[0];
  const hasMenu = methods.length > 1;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const tooltipFor = (m: DownloadMethod) =>
    m.kind === "torrent" ? t("result.torrent_tooltip") : t("result.mirror_tooltip");

  return (
    <div ref={ref} className="relative ml-auto inline-flex shrink-0">
      <button
        onClick={() => onPick(primary)}
        title={`${primary.name} — ${tooltipFor(primary)}`}
        className={`inline-flex items-center gap-1.5 bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 transition-colors hover:bg-brand-400 ${
          hasMenu ? "rounded-l-lg" : "rounded-lg"
        }`}
      >
        <Download className="h-3.5 w-3.5" />
        {t("result.download")}
      </button>
      {hasMenu && (
        <button
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={open}
          title={t("result.more_methods")}
          aria-label={t("result.more_methods")}
          className="inline-flex items-center rounded-r-lg border-l border-slate-950/20 bg-brand-500 px-1.5 text-slate-950 transition-colors hover:bg-brand-400"
        >
          <ChevronDown className="h-3.5 w-3.5" />
        </button>
      )}
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 z-20 mb-1.5 w-72 overflow-hidden rounded-lg border border-sf-border bg-sf-surface py-1 shadow-xl shadow-black/40"
        >
          {methods.map((m, i) => (
            <button
              key={i}
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onPick(m);
              }}
              title={tooltipFor(m)}
              className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-xs text-sf-text hover:bg-sf-raised"
            >
              <Download
                className={`h-3.5 w-3.5 shrink-0 ${
                  m.kind === "torrent" ? "text-brand-300" : "text-sf-muted"
                }`}
              />
              <span className="min-w-0 flex-1 truncate">{m.name}</span>
              <span className="shrink-0 text-[11px] uppercase tracking-wide text-sf-muted">
                {m.kind === "torrent" ? "Torrent" : "Mirror"}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Etiquetas sobre la foto con colores fijos (bg-black/…, hex): deben leerse
 *  igual sobre cualquier imagen y en ambos temas; los overrides del tema
 *  claro no tocan estas clases. */
function Banner({
  imageUrl,
  alt,
  icao,
  simulator,
  installState,
  hasGsxProfile,
}: {
  imageUrl: string | null;
  alt: string;
  icao: string | null;
  simulator: string;
  installState: InstallState;
  hasGsxProfile: boolean;
}) {
  const [failed, setFailed] = useState(false);
  const showImage = imageUrl && !failed;

  return (
    <div className="relative aspect-video w-full overflow-hidden rounded-t-xl bg-gradient-to-br from-slate-800 via-slate-900 to-slate-950">
      {showImage ? (
        <img
          src={imageUrl}
          alt={alt}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
          className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.03]"
        />
      ) : (
        <BannerPlaceholder icao={icao} />
      )}

      <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950/60 via-transparent to-slate-950/40" />

      <div className="absolute inset-x-0 top-0 flex flex-wrap items-start justify-between gap-1.5 p-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {icao && (
            <span className="inline-flex items-center gap-1 rounded-md bg-black/65 px-2 py-0.5 font-mono text-xs font-semibold text-[#a6eedf] ring-1 ring-[#35cdb2]/40">
              <Plane className="h-3 w-3" />
              {icao}
            </span>
          )}
          <span className="rounded-md bg-black/65 px-2 py-0.5 text-[11px] font-medium text-white/90 ring-1 ring-white/15">
            {simulator}
          </span>
          {hasGsxProfile && (
            <span
              className="inline-flex items-center gap-1 rounded-md bg-violet-500/25 px-2 py-0.5 text-[11px] font-semibold text-violet-100 ring-1 ring-violet-500/50"
              title={t("result.gsx_installed_tooltip")}
            >
              <CheckCircle2 className="h-3 w-3" />
              GSX
            </span>
          )}
        </div>
        <InstallBadge state={installState} />
      </div>
    </div>
  );
}

/**
 * Badge en la esquina superior derecha del banner. Su color y texto
 * resumen el estado del addon respecto al Community local:
 *   · Update disponible → ámbar prominente con flecha de versión.
 *   · Al día            → verde tenue con check.
 *   · Instalado (versión desconocida) → verde con texto neutro.
 *   · No instalado      → no se renderiza nada.
 */
function InstallBadge({ state }: { state: InstallState }) {
  if (state.kind === "not-installed") return null;

  if (state.kind === "update") {
    return (
      <span
        className="inline-flex items-center gap-1 rounded-md bg-amber-500 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide text-amber-950 shadow-lg shadow-amber-500/30"
        title={t("result.update.tooltip", {
          installed: state.pkg.packageVersion ?? "?",
          latest: state.latestVersion,
        })}
      >
        <Sparkles className="h-3 w-3" />
        {t("result.update.label", {
          from: state.pkg.packageVersion ?? "?",
          to: state.latestVersion,
        })}
      </span>
    );
  }

  if (state.kind === "up-to-date") {
    return (
      <span
        className="inline-flex items-center gap-1 rounded-md bg-[#10b981] px-2 py-0.5 text-[11px] font-semibold text-[#022c22] shadow-lg shadow-black/30"
        title={t("result.installed.with_version.tooltip", {
          version: state.pkg.packageVersion ?? "",
        })}
      >
        <CheckCircle2 className="h-3 w-3" />
        {t("result.installed.with_version", {
          version: state.pkg.packageVersion ?? "",
        })}
      </span>
    );
  }

  // installed-unknown-version
  return (
    <span
      className="inline-flex items-center gap-1 rounded-md bg-[#10b981] px-2 py-0.5 text-[11px] font-semibold text-[#022c22] shadow-lg shadow-black/30"
      title={t("result.installed.unknown.tooltip")}
    >
      <CheckCircle2 className="h-3 w-3" />
      {t("result.installed.unknown")}
    </span>
  );
}

function BannerPlaceholder({ icao }: { icao: string | null }) {
  return (
    <div className="flex h-full w-full items-center justify-center">
      <div className="flex flex-col items-center gap-2 text-slate-600">
        <Plane className="h-9 w-9" strokeWidth={1.5} />
        {icao && (
          <span className="font-mono text-2xl font-bold tracking-widest text-slate-600">
            {icao}
          </span>
        )}
      </div>
    </div>
  );
}
