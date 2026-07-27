import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import {
  X,
  Download,
  Star,
  ExternalLink,
  Truck,
  Flame,
  CircleOff,
} from "lucide-react";
import { useGsxOfferStore } from "../stores/useGsxOfferStore";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { GsxProfile } from "../lib/types";

/** Palabras genéricas que NO sirven para casar el desarrollador del escenario
 *  con el perfil (aparecen en muchos nombres). */
const GENERIC = new Set([
  "design",
  "group",
  "studios",
  "studio",
  "scenery",
  "sceneries",
  "airport",
  "airports",
  "simulation",
  "simulations",
  "software",
  "interactive",
  "entertainment",
  "the",
  "and",
  "aero",
  "sim",
  "sims",
  "msfs",
  "pro",
  "for",
  "gsx",
  "profile",
  "profiles",
]);

const normAlnum = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

/** ¿El perfil GSX es del mismo desarrollador que el escenario instalado?
 *  Casamos por varias vías para cubrir nombres reales:
 *   · nombre completo normalizado ("MK-STUDIOS" → "mkstudios") dentro del título
 *     ("[MKStudios] Rome Fiumicino…") — y su stem sin la "s" final ("mkstudio",
 *     que cubre "MK Studio");
 *   · sigla de iniciales ≥3 ("Sin Design Group" → "sdg", que aparece como "SDG");
 *   · palabras propias distintivas (≥4, no genéricas), p.ej. "FlyTampa". */
function isCompatible(profileTitle: string, creator: string | null): boolean {
  if (!creator) return false;
  const title = normAlnum(profileTitle);
  if (!title) return false;
  const cn = normAlnum(creator);
  if (cn.length >= 4 && title.includes(cn)) return true;
  const stem = cn.endsWith("s") ? cn.slice(0, -1) : cn;
  if (stem.length >= 4 && title.includes(stem)) return true;
  const words = creator.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  if (words.length >= 2) {
    const abbr = words.map((w) => w[0]).join("");
    if (abbr.length >= 3 && title.includes(abbr)) return true;
  }
  return words.some((w) => w.length >= 4 && !GENERIC.has(w) && title.includes(w));
}

/**
 * (v6.2.66) Modal de perfiles GSX al instalar un escenario de aeropuerto.
 * Siempre se muestra: si hay perfiles, los pinta en un GRID de tarjetas (nombre,
 * imagen, descargas, estrellas) marcando el MÁS POPULAR (más descargas) y el
 * COMPATIBLE (su título casa con el desarrollador del escenario). Clic en una
 * tarjeta → abre esa página en el embebido. Si NO hay perfiles, muestra un
 * mensaje indicando el aeropuerto.
 */
export function GsxOfferModal() {
  const offer = useGsxOfferStore((s) => s.offer);
  const dismiss = useGsxOfferStore((s) => s.dismiss);

  const openProfile = (link: string) => {
    void api.openLiveryBrowser(link);
    dismiss();
  };

  const profiles = offer?.profiles ?? [];
  const maxDl = Math.max(0, ...profiles.map((p) => p.downloads ?? 0));
  const decorate = (p: GsxProfile) => ({
    p,
    // "Más popular" sólo tiene sentido comparando: nunca lo marcamos si hay un
    // único perfil para el aeropuerto.
    popular: profiles.length >= 2 && maxDl > 0 && (p.downloads ?? 0) === maxDl,
    compatible: isCompatible(p.title, offer?.sceneryCreator ?? null),
  });
  // Compatibles primero, luego por descargas desc.
  const cards = profiles
    .map(decorate)
    .sort(
      (a, b) =>
        Number(b.compatible) - Number(a.compatible) ||
        (b.p.downloads ?? 0) - (a.p.downloads ?? 0),
    );

  return createPortal(
    <AnimatePresence>
      {offer && (
        <motion.div
          className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={dismiss}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.97, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 8 }}
            transition={{ duration: 0.14 }}
            onClick={(e) => e.stopPropagation()}
            className="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
          >
            <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-4">
              <div className="flex min-w-0 items-start gap-3">
                <Truck className="mt-0.5 h-5 w-5 shrink-0 text-cyan-300" />
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold text-slate-100">
                    {t("gsxoffer.title", { icao: offer.icao })}
                  </h3>
                  <p className="mt-0.5 truncate text-xs text-slate-400">
                    {offer.airportTitle}
                    {profiles.length > 0
                      ? ` · ${t("gsxoffer.subtitle", { n: String(profiles.length) })}`
                      : ""}
                  </p>
                </div>
              </div>
              <button
                onClick={dismiss}
                title={t("common.dismiss")}
                className="shrink-0 rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            {cards.length === 0 ? (
              <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 overflow-y-auto px-6 py-14 text-center">
                <CircleOff className="h-8 w-8 text-slate-600" />
                <p className="text-sm text-slate-300">
                  {t("gsxoffer.empty", { airport: offer.airportTitle })}
                </p>
                <p className="text-xs text-slate-500">
                  {t("gsxoffer.empty_icao", { icao: offer.icao })}
                </p>
              </div>
            ) : (
              <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-y-auto p-4 sm:grid-cols-2">
                {cards.map(({ p, popular, compatible }) => (
                  <button
                    key={p.id}
                    onClick={() => openProfile(p.link)}
                    className="group flex flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-900/50 text-left transition-colors hover:border-cyan-500/40 hover:bg-slate-900"
                  >
                    <div className="relative aspect-[16/9] w-full shrink-0 overflow-hidden bg-slate-800">
                      {p.thumbnail && (
                        <img
                          src={p.thumbnail}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.03]"
                          onError={(e) => {
                            (e.currentTarget as HTMLImageElement).style.display =
                              "none";
                          }}
                        />
                      )}
                      {/* Etiquetas superpuestas: Más popular (ámbar + llama) y
                          Compatible (esmeralda). Pueden salir ambas. */}
                      <div className="absolute left-2 top-2 flex flex-wrap gap-1">
                        {popular && (
                          <span className="inline-flex items-center gap-1 rounded bg-amber-500 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-amber-950 shadow">
                            <Flame className="h-3 w-3" />
                            {t("gsxoffer.most_popular")}
                          </span>
                        )}
                        {compatible && (
                          <span className="inline-flex items-center rounded bg-emerald-500 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-emerald-950 shadow">
                            {t("gsxoffer.compatible")}
                          </span>
                        )}
                      </div>
                      <ExternalLink className="absolute right-2 top-2 h-4 w-4 text-white/70 drop-shadow" />
                    </div>
                    <div className="flex flex-1 flex-col p-3">
                      <div className="line-clamp-2 text-sm font-medium text-slate-100">
                        {p.title}
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-slate-400">
                        <span className="inline-flex items-center gap-1">
                          <Download className="h-3 w-3" />
                          {(typeof p.downloads === "number"
                            ? p.downloads
                            : 0
                          ).toLocaleString()}
                        </span>
                        <span className="inline-flex items-center gap-1 text-amber-300">
                          <Star className="h-3 w-3 fill-amber-300" />
                          {typeof p.rating === "number" && p.rating > 0
                            ? `${p.rating.toFixed(1)}${p.ratingCount ? ` (${p.ratingCount})` : ""}`
                            : "—"}
                        </span>
                      </div>
                      {p.authorName && (
                        <div className="mt-1 truncate text-[11px] text-slate-500">
                          {p.authorName}
                        </div>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            )}

            <footer className="border-t border-slate-800 px-5 py-2 text-center text-[11px] text-slate-500">
              {t("gsxoffer.hint")}
            </footer>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
