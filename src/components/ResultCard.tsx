import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  Calendar,
  CheckCircle2,
  Download,
  ExternalLink,
  FileText,
  Plane,
  Sparkles,
  Tag,
  User,
} from "lucide-react";
import type { Addon, CommunityPackage, DownloadMethod } from "../lib/types";
import { api } from "../lib/tauri";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { isNewer } from "../lib/version";
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

/**
 * Normaliza un título para compararlo con el folder/title del
 * manifest. Quita versión, sufijos `MSFS 2020`, paréntesis con
 * etiquetas tipo `(Merged)` / `(Converted)` para usarlas como
 * señal pero sin que rompan el match exacto.
 */
function normalizeTitle(s: string): string {
  return s
    .toLowerCase()
    .replace(/\sv\d+(?:\.\d+)+\b.*$/i, "")
    .replace(/\bmsfs\s*\d{4}(?:\/\d+)?\b/gi, "")
    .replace(/[\s ]+/g, " ")
    .trim();
}

/** (v5.4.1) Colapsa sub-marcas de desarrollador a un token canónico
 *  (solo alfanuméricos). Algunas casas publican la escenografía bajo otra
 *  marca: el manifest trae "iniScene" pero el catálogo lista "iniBuilds" —
 *  misma casa. Sin esto KLAX (iniScene) no se marcaba instalado. */
function canonDev(s: string | null | undefined): string {
  const n = (s ?? "").replace(/[^a-z0-9]/gi, "").toLowerCase();
  if (n === "iniscene") return "inibuilds";
  return n;
}

/** True si dos cadenas (creator del manifest y developer del
 *  catálogo) corresponden plausiblemente al mismo dueño.
 *  Substring case-insensitive — tolera "Aerosoft" vs "Aerosoft GmbH". */
function sameCreator(a: string | null | undefined, b: string | null | undefined): boolean {
  const aa = canonDev(a);
  const bb = canonDev(b);
  if (!aa || !bb) return false;
  return aa.includes(bb) || bb.includes(aa);
}

/** Extrae identificadores de modelo de aeronave del texto.
 *  Cubre Airbus (A320/A330/A350/A380…), Boeing (737/747/757/767/777/787),
 *  variantes con sufijos (-200, -300ER, -800), y modelos GA/regionales
 *  comunes (CRJ, ATR, MD-11, DC-6, TBM930, C172).
 *
 *  El usuario reportó: "tengo un A350 instalado, busco A350 en
 *  Simplaza, no me marca instalado". Causa: el catálogo de
 *  Simplaza decía "Airbus A350" pero el manifest del paquete
 *  decía "iniBuilds A350-900" — el substring de título no
 *  coincidía. Extrayendo el modelo común (A350) ambos hacen match.
 */
function aircraftModels(text: string): string[] {
  const out: string[] = [];
  const re =
    /\b(A\d{3}(?:-?\d{3})?|B\d{3}(?:-?\d{3}[A-Z]{0,3})?|73[6-9](?:-?\d{3})?|74[478](?:-?\d{1,3})?|76\d|77\d(?:-?\d{3}[A-Z]{0,3})?|78\d|CRJ\d?\d?|ATR-?\d{2}\d?|MD-?\d{2}|DC-?\d|TBM\d{3}|C\d{3,4}|CJ\d|Q\d{3})\b/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    // Normalizamos: "A350-900" → "A350900"; "737-800" → "737800";
    // sin guiones para que el match sea robusto (la fuente y el
    // manifest pueden o no incluir el guión).
    out.push(m[1].toUpperCase().replace(/-/g, ""));
  }
  return out;
}

/** True si ambos textos comparten al menos un identificador de
 *  modelo de aeronave (A350, B777, etc.). Tolerante a sufijos:
 *  "A350" matchea con "A350-900" porque tras normalización ambos
 *  contienen "A350" como prefijo. */
function shareAircraftModel(a: string, b: string): boolean {
  const am = aircraftModels(a);
  if (am.length === 0) return false;
  const bm = aircraftModels(b);
  if (bm.length === 0) return false;
  for (const x of am) {
    for (const y of bm) {
      // Match exacto o por prefijo (A350 ⊂ A350900).
      if (x === y || x.startsWith(y) || y.startsWith(x)) return true;
    }
  }
  return false;
}

/** True si el nombre delata un addon "categoría" (livery, sound,
 *  preset, paint, etc.) — usado para el filtro asimétrico que
 *  evita marcar como "instalado/update disponible" un livery pack
 *  cuando lo que el usuario tiene es el avión base. */
function isCategoryAddonName(s: string): boolean {
  return /\b(liver(y|ies)|paint|texture|sound\s*pack|sound\b|preset|profile|config|tweak|enhancement|\bmod\b|\bpack\b|repaint)/i.test(
    s,
  );
}

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
  // Match consistente con el detector backend:
  //   A. SCENERY → ICAO + creator/developer.
  //   B. AIRCRAFT/livery/etc → creator/developer + título (substring).
  //
  // Un match estricto por creator evita falsos "Instalado" cuando
  // hay dos sceneries del mismo ICAO de devs distintos (Aerosoft
  // EBBR + JustSim EBBR — solo el realmente instalado debe marcarse).

  let exactMatch: CommunityPackage | undefined;

  // Camino A — ICAO + creator
  if (addon.icao) {
    const target = addon.icao.toUpperCase();
    const sameIcao = packages.filter(
      (p) => p.icao && p.icao.toUpperCase() === target,
    );
    if (sameIcao.length > 0 && addon.developer) {
      exactMatch = sameIcao.find((p) => sameCreator(p.creator, addon.developer));
    }
    if (!exactMatch) {
      // Fallback por título normalizado — útil cuando el manifest
      // del paquete instalado no expone creator parseable. PERO:
      // si ambos lados tienen developer y son DISTINTOS, NO es
      // match. El bug reportado por el usuario: tenía RCTP de
      // ACO Design Studio v1.2.0 instalado, y el card del RCTP
      // de Double T en el catálogo aparecía como "Instalado" sólo
      // porque normalizeTitle(ACO) === normalizeTitle(Double T).
      // Asimetría: si los devs existen y difieren → distintos
      // productos del mismo aeropuerto.
      const addonNorm = normalizeTitle(addon.name);
      exactMatch = sameIcao.find((p) => {
        if (normalizeTitle(p.title) !== addonNorm) return false;
        if (
          addon.developer &&
          p.creator &&
          !sameCreator(p.creator, addon.developer)
        ) {
          return false;
        }
        return true;
      });
    }
  }

  // Camino B — creator + (título substring O modelo de aeronave
  // compartido). El match por modelo cubre el caso "iniBuilds –
  // Airbus A350" del catálogo vs "iniBuilds A350-900" del manifest:
  // el creator matchea por substring (iniBuilds ⊂ iniBuilds Limited)
  // y ambos contienen el modelo A350.
  //
  // **Filtro asimétrico anti-categoría** (v0.1.10, mismo principio
  // que el SQL en compute_available): si el addon del catálogo es
  // de otra "categoría" (livery, sound, preset, paint, mod, pack)
  // pero el paquete instalado NO lo es, NO matchea — un avión
  // base no se "actualiza" con un livery pack. Bug reportado
  // por usuario: tenía PMDG 737-800 base y aparecía como UPDATE
  // disponible "Boeing 737-700/800/900 Liveries by PMDG".
  if (!exactMatch && addon.developer) {
    exactMatch = packages.find((p) => {
      if (!sameCreator(p.creator, addon.developer)) return false;
      const a = addon.name.toLowerCase();
      const b = p.title.toLowerCase();
      const titlesOverlap =
        a.includes(b) ||
        b.includes(a) ||
        shareAircraftModel(addon.name, p.title);
      if (!titlesOverlap) return false;
      // Asimetría: a (catálogo) contiene categoría, b (instalado) no → reject.
      if (isCategoryAddonName(a) && !isCategoryAddonName(b)) return false;
      return true;
    });
  }

  // **No hay Camino C para AIRCRAFT/livery.** Antes intentábamos
  // detectar por modelo sólo (sin creator) para Simplaza — pero eso
  // daba falsos positivos masivos: buscar "737" marcaba como instalados
  // todos los addons de 737 (FS2Crew SOP, MSFS Wear, repaints…)
  // porque el usuario tenía PMDG 737-800 base. La heurística por
  // modelo sin creator es inherentemente ambigua: dos productos
  // distintos pueden ser para el mismo avión.
  //
  // **(v3.7.0 fix O1) Camino D — SCENERY con ICAO único**:
  // si el addon es de un aeropuerto (tiene ICAO) y el usuario tiene
  // **exactamente UN** paquete con ese ICAO en Community, lo marcamos
  // como instalado aunque el catálogo no exponga developer parseable
  // (caso KATL Imagine Simulation: el catálogo devolvía addon.developer
  // null y title largo, los Caminos A/B fallaban). El riesgo de falso
  // positivo es bajo: pilotos rara vez tienen 2 sceneries del mismo
  // aeropuerto instalados al mismo tiempo. Si tienen 2+, el match
  // estricto sigue activo (los multi-ICAO se manejan en Camino A).
  if (!exactMatch && addon.icao) {
    const target = addon.icao.toUpperCase();
    const sameIcao = packages.filter(
      (p) => p.icao && p.icao.toUpperCase() === target,
    );
    if (sameIcao.length === 1) {
      const only = sameIcao[0];
      // (v5.0.0) BLINDAJE por creador: el Camino D existía para el caso
      // de catálogo SIN developer parseable. Pero si el catálogo SÍ
      // declara un developer y NO coincide con el del paquete instalado,
      // es OTRO producto del mismo aeropuerto — no lo marcamos. Bug
      // reportado: con Aerosoft ENGM instalado, las fichas de Orbx y
      // JustSim (mismo ICAO) aparecían como "Installed/Update".
      const devConflict =
        !!addon.developer &&
        !!only.creator &&
        !sameCreator(only.creator, addon.developer);
      if (!devConflict) {
        exactMatch = only;
      }
    }
  }

  if (!exactMatch) return { kind: "not-installed" };

  const pkg = exactMatch;

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
  const hasTorrent = addon.downloadMethods.some((m) => m.kind === "torrent");
  const hasMirror  = addon.downloadMethods.some((m) => m.kind === "mirror");
  const startDownload = useDownloadsStore((s) => s.start);
  const communityPackages = useCommunityStore((s) => s.packages);
  const gsxInstalledIcaos = useGsxLocalStore((s) => s.installedIcaos);
  // Changelog modal — sólo lo exponemos en Simplaza (lo pidió el
  // usuario tras quitarlo del modal de detalle de paquete instalado).
  // SceneryAddons rara vez incluye changelog útil en sus posts y el
  // botón quedaría siempre vacío.
  const [showChangelog, setShowChangelog] = useState(false);
  const isSimplaza = addon.source === "simplaza";

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
  // borde slate = no instalado.
  const cardBorder =
    installState.kind === "update"
      ? "border-amber-500/50 hover:border-amber-400/70"
      : installState.kind === "up-to-date" || installState.kind === "installed-unknown-version"
        ? "border-emerald-500/30 hover:border-emerald-400/50"
        : "border-slate-800 hover:border-brand-500/40";

  const handleDownload = (method: DownloadMethod) => {
    startDownload({
      addonId: addon.id,
      addonTitle: addon.name,
      source: addon.source,
      method,
      addonSimulator: addon.simulator,
      // (v7.1) Todos los métodos → si el torrent falla, reofrecemos mirrors.
      allMethods: addon.downloadMethods,
    });
  };

  return (
    <motion.article
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18 }}
      className={`group overflow-hidden rounded-2xl border bg-slate-900/50 backdrop-blur transition-colors hover:bg-slate-900/70 ${cardBorder}`}
    >
      {/* Banner superior — ICAO + simulator como overlays sobre la
          imagen. El badge GSX (descarga) vivía aquí antes pero migró
          a un control único debajo del SearchBar. (v1.1.4) Ahora
          mostramos un mini-check si el usuario tiene perfil GSX
          local instalado para este ICAO. */}
      <Banner
        imageUrl={addon.imageUrl}
        alt={addon.name}
        icao={addon.icao}
        simulator={addon.simulator}
        installState={installState}
        hasGsxProfile={hasGsxProfile}
      />

      <div className="p-5">
        <header className="flex items-start justify-between gap-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-base font-semibold leading-snug text-slate-100">
              {addon.name}
            </h3>
            <div className="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-slate-400">
              {addon.developer && (
                <span className="inline-flex items-center gap-1">
                  <User className="h-3.5 w-3.5" />
                  {addon.developer}
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
          </div>
          <button
            onClick={() => api.openExternal(addon.pageUrl)}
            className="shrink-0 rounded-lg p-2 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
            title={t("result.open_source")}
          >
            <ExternalLink className="h-4 w-4" />
          </button>
        </header>

        <footer className="mt-4 flex flex-wrap items-center gap-2">
          {hasTorrent && <Chip label="Torrent" tone="brand" />}
          {hasMirror  && <Chip label="Mirror" tone="slate" />}
          {addon.downloadMethods.length === 0 && (
            <span className="text-xs text-slate-500">{t("result.no_download_methods")}</span>
          )}
          <div className="ml-auto flex flex-wrap gap-2">
            {isSimplaza && (
              <button
                onClick={() => setShowChangelog(true)}
                title={t("result.view_simplaza_changelog")}
                className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-900/40 px-3 py-1.5 text-xs font-medium text-slate-300 transition-colors hover:border-brand-500/40 hover:bg-slate-800 hover:text-slate-100"
              >
                <FileText className="h-3.5 w-3.5" />
                {t("result.changelog")}
              </button>
            )}
            {addon.downloadMethods.slice(0, 3).map((m, i) => (
              <button
                key={i}
                onClick={() => handleDownload(m)}
                className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors ${
                  m.kind === "torrent"
                    ? "border-brand-500/50 bg-brand-500/10 text-brand-200 hover:border-brand-400 hover:bg-brand-500/20"
                    : "border-slate-700 bg-slate-800/80 text-slate-200 hover:border-brand-500/60 hover:bg-slate-800"
                }`}
                title={
                  m.kind === "torrent"
                    ? t("result.torrent_tooltip")
                    : t("result.mirror_tooltip")
                }
              >
                <Download className="h-3.5 w-3.5" />
                {m.name}
              </button>
            ))}
          </div>
        </footer>
      </div>

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
    <div className="relative aspect-[21/9] w-full overflow-hidden bg-gradient-to-br from-slate-800 via-slate-900 to-slate-950">
      {showImage ? (
        <img
          src={imageUrl}
          alt={alt}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
          className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.02]"
        />
      ) : (
        <BannerPlaceholder icao={icao} />
      )}

      <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950/80 via-transparent to-slate-950/40" />

      <div className="absolute inset-x-0 top-0 flex flex-wrap items-start justify-between gap-1.5 p-3">
        <div className="flex flex-wrap items-center gap-1.5">
          {icao && (
            <span className="inline-flex items-center gap-1 rounded-md bg-slate-950/80 px-2 py-1 font-mono text-xs font-semibold text-brand-300 ring-1 ring-brand-500/30 backdrop-blur">
              <Plane className="h-3 w-3" />
              {icao}
            </span>
          )}
          <span className="rounded-md bg-slate-950/80 px-2 py-1 text-[11px] uppercase tracking-wide text-slate-300 ring-1 ring-slate-700 backdrop-blur">
            {simulator}
          </span>
          {hasGsxProfile && (
            <span
              className="inline-flex items-center gap-1 rounded-md bg-violet-500/15 px-2 py-1 text-[11px] font-semibold text-violet-200 ring-1 ring-violet-500/40 backdrop-blur"
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
        className="inline-flex items-center gap-1 rounded-md bg-amber-500/95 px-2 py-1 text-[11px] font-bold uppercase tracking-wide text-amber-950 shadow-lg shadow-amber-500/30 ring-1 ring-amber-300 backdrop-blur"
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
        className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[11px] font-semibold text-emerald-200 ring-1 ring-emerald-500/40 backdrop-blur"
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
      className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[11px] font-semibold text-emerald-200 ring-1 ring-emerald-500/40 backdrop-blur"
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
      <div className="flex flex-col items-center gap-2 text-slate-700">
        <Plane className="h-10 w-10" strokeWidth={1.5} />
        {icao && (
          <span className="font-mono text-3xl font-bold tracking-widest text-slate-700">
            {icao}
          </span>
        )}
      </div>
    </div>
  );
}

function Chip({ label, tone }: { label: string; tone: "brand" | "slate" }) {
  const toneClass =
    tone === "brand"
      ? "bg-brand-500/15 text-brand-300 ring-1 ring-brand-500/30"
      : "bg-slate-700/40 text-slate-300 ring-1 ring-slate-700";
  return (
    <span className={`rounded-md px-2 py-0.5 text-xs ${toneClass}`}>{label}</span>
  );
}
