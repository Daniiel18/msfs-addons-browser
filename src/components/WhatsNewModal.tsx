import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import {
  Plane,
  Download,
  ShieldCheck,
  AppWindow,
  ArrowDownUp,
  FolderTree,
  Medal,
  Gamepad2,
  CloudDownload,
  Palette,
  PlaneTakeoff,
  Layers,
  MapPin,
  type LucideIcon,
} from "lucide-react";
import { getActiveLocale } from "../lib/i18n";

/**
 * (v7) "Novedades" — modal BLOQUEANTE que se muestra UNA vez por versión al
 * arrancar. A diferencia del carrusel viejo: NO tiene X ni se cierra al hacer
 * clic afuera ni con Esc — el usuario DEBE leerlo. Las novedades se reparten en
 * páginas de 4 (paginado dinámico) y cada página tiene una cuenta atrás corta
 * antes de poder avanzar, para que no se salte de un solo clic. Contenido
 * bilingüe embebido (sin claves i18n, para iterar rápido).
 *
 * (v7.2.0) Se AÑADEN las novedades nuevas al final, conservando las anteriores
 * — el usuario ve todo lo nuevo hasta ahora.
 */
const WHATS_NEW_VERSION = "7.2.0";
const SEEN_KEY = "simfleet.whatsnew.seen.v4";
const PER_PAGE_SECONDS = 3;

type Feat = { Icon: LucideIcon; es: [string, string]; en: [string, string] };

const FEATURES: Feat[] = [
  {
    Icon: Download,
    es: ["Descarga directa de mirrors", "modsfire, mixdrop y otros se bajan e instalan sin salir de SimFleet."],
    en: ["Direct mirror downloads", "modsfire, mixdrop and more download and install without leaving SimFleet."],
  },
  {
    Icon: ShieldCheck,
    es: ["Bloqueador de anuncios", "uBlock Origin Lite dentro del navegador de descargas — adiós a los pop-ups."],
    en: ["Built-in ad blocker", "uBlock Origin Lite inside the download browser — no more pop-ups."],
  },
  {
    Icon: AppWindow,
    es: ["Ventana de descarga tipo popout", "Con botón para cancelar la descarga y sin los botones del sistema."],
    en: ["Pop-out download window", "With a cancel button and none of the system browser chrome."],
  },
  {
    Icon: ArrowDownUp,
    es: ["Mirrors más confiables", "modsfire y mixdrop primero, y si un torrent se queda sin seeds te ofrecemos los mirrors al instante."],
    en: ["More reliable mirrors", "modsfire and mixdrop first, and if a torrent has no seeds we offer you the mirrors instantly."],
  },
  {
    Icon: FolderTree,
    es: ["Perfiles GSX agrupados por variante", "El instalador separa FULL, LIGHT y las demás variantes por carpeta."],
    en: ["GSX profiles grouped by variant", "The installer splits FULL, LIGHT and other variants into folders."],
  },
  {
    Icon: Medal,
    es: ["Logros con medallas", "23 insignias con niveles (bronce a diamante) sobre tu historial de vuelos."],
    en: ["Achievements with medals", "23 tiered badges (bronze to diamond) based on your flight history."],
  },
  {
    Icon: Gamepad2,
    es: ["Discord Rich Presence", "Muestra tu vuelo en tu perfil de Discord (opcional, apagado por defecto)."],
    en: ["Discord Rich Presence", "Shows your flight on your Discord profile (optional, off by default)."],
  },
  {
    Icon: CloudDownload,
    es: ["Integración con flightsim desde SimFleet", "Tus addons de flightsim.to dentro de SimFleet, con aviso cuando hay actualización."],
    en: ["flightsim.to integration inside SimFleet", "Your flightsim.to addons inside SimFleet, with update alerts."],
  },
  // ---- (v7.2.0) Novedades nuevas ----
  {
    Icon: Palette,
    es: ["Get Liveries por avión", "Cada avión tiene un botón que abre sus liveries en flightsim.to — detecta el modelo automáticamente, hasta para aviones nuevos."],
    en: ["Get Liveries per aircraft", "Every aircraft has a button that opens its liveries on flightsim.to — it detects the model automatically, even for new aircraft."],
  },
  {
    Icon: PlaneTakeoff,
    es: ["Filtro Aircraft más preciso", "Aircraft muestra solo aviones de verdad — incluidos los de estudio encriptados (PMDG, Fenix, iniBuilds) — y separa liveries y mods a su categoría."],
    en: ["Sharper Aircraft filter", "Aircraft shows only real aircraft — even encrypted study-level ones (PMDG, Fenix, iniBuilds) — and moves liveries and mods to their own category."],
  },
  {
    Icon: Layers,
    es: ["Compatibilidad MSFS 2020 / 2024", "Al instalar respeta si el addon es para 2020, 2024 o ambos — sin ofrecerte el simulador equivocado."],
    en: ["MSFS 2020 / 2024 compatibility", "When installing, it respects whether an add-on is for 2020, 2024 or both — no wrong-sim offers."],
  },
  {
    Icon: MapPin,
    es: ["Aeropuertos mejor detectados", "El ICAO se lee de la estructura real del escenario (BGL/carpeta), no de una palabra del nombre — perfiles GSX correctos."],
    en: ["Airports detected right", "The ICAO is read from the scenery's real structure (BGL/folder), not from a word in its name — correct GSX profiles."],
  },
];

// Paginado dinámico: 4 novedades por página.
const PAGES: Feat[][] = [];
for (let i = 0; i < FEATURES.length; i += 4) {
  PAGES.push(FEATURES.slice(i, i + 4));
}

/** (v7) ¿El usuario todavía no vio las novedades de esta versión? Lo usa
 *  `App.tsx` para que el tour de bienvenida ESPERE a que este modal bloqueante
 *  se cierre — si no, en una instalación nueva ambos salen a la vez y el backdrop
 *  del tour tapa el botón «Continuar», dejando la app en soft-lock. */
export function isWhatsNewPending(): boolean {
  try {
    return localStorage.getItem(SEEN_KEY) !== WHATS_NEW_VERSION;
  } catch {
    return false;
  }
}

/** Evento que dispara el modal al cerrarse, para encadenar el tour. */
export const WHATS_NEW_CLOSED_EVENT = "simfleet:whatsnew-closed";

const UI = {
  es: { title: "Novedades de SimFleet", next: "Siguiente", done: "Empezar" },
  en: { title: "What's new in SimFleet", next: "Next", done: "Get started" },
} as const;

export function WhatsNewModal() {
  const [open, setOpen] = useState(false);
  const [page, setPage] = useState(0);
  const [left, setLeft] = useState(PER_PAGE_SECONDS);

  const locale = getActiveLocale();
  const ui = UI[locale] ?? UI.en;

  // Mostrar UNA vez por versión: si el usuario aún no vio esta versión, abrir.
  useEffect(() => {
    try {
      if (localStorage.getItem(SEEN_KEY) !== WHATS_NEW_VERSION) setOpen(true);
    } catch {
      /* ignore */
    }
  }, []);

  // Cuenta atrás por página: se reinicia al abrir y al cambiar de página.
  useEffect(() => {
    if (!open) return;
    setLeft(PER_PAGE_SECONDS);
    const id = setInterval(() => {
      setLeft((n) => {
        if (n <= 1) {
          clearInterval(id);
          return 0;
        }
        return n - 1;
      });
    }, 1000);
    return () => clearInterval(id);
  }, [open, page]);

  const isLast = page === PAGES.length - 1;
  const waiting = left > 0;

  const finish = () => {
    try {
      localStorage.setItem(SEEN_KEY, WHATS_NEW_VERSION);
    } catch {
      /* ignore */
    }
    setOpen(false);
    // Avisa a App.tsx que ya se cerró → recién ahí abre el tour de bienvenida.
    try {
      window.dispatchEvent(new Event(WHATS_NEW_CLOSED_EVENT));
    } catch {
      /* ignore */
    }
  };

  const advance = () => {
    if (waiting) return;
    if (isLast) finish();
    else setPage((p) => p + 1);
  };

  const btnLabel = (isLast ? ui.done : ui.next) + (waiting ? ` (${left})` : "");

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-4"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
        >
          {/* Sin onClick de cierre: el fondo es un bloqueo, no una salida. */}
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.16 }}
            role="dialog"
            aria-modal="true"
            className="w-full max-w-md rounded-2xl border border-slate-800 bg-slate-900 p-7 text-center shadow-2xl ring-1 ring-slate-800"
          >
            <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-sky-500 text-white">
              <Plane className="h-8 w-8" />
            </div>
            <h2 className="mb-6 text-xl font-semibold text-slate-100">{ui.title}</h2>

            <AnimatePresence mode="wait">
              <motion.ul
                key={page}
                initial={{ opacity: 0, x: 14 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -14 }}
                transition={{ duration: 0.18 }}
                className="flex flex-col gap-4 text-left"
              >
                {PAGES[page].map((f) => {
                  const [title, desc] = locale === "es" ? f.es : f.en;
                  return (
                    <li key={title} className="flex gap-3">
                      <f.Icon className="mt-0.5 h-5 w-5 shrink-0 text-sky-300" />
                      <div>
                        <p className="text-sm font-medium text-slate-100">{title}</p>
                        <p className="mt-0.5 text-xs leading-relaxed text-slate-400">{desc}</p>
                      </div>
                    </li>
                  );
                })}
              </motion.ul>
            </AnimatePresence>

            <div className="mt-6 flex items-center justify-center gap-1.5">
              {PAGES.map((_, n) => (
                <span
                  key={n}
                  className={`h-1.5 rounded-full transition-all ${
                    n === page ? "w-5 bg-sky-400" : "w-1.5 bg-slate-700"
                  }`}
                />
              ))}
            </div>

            <button
              onClick={advance}
              disabled={waiting}
              className={`mt-5 inline-flex h-11 min-w-[150px] items-center justify-center rounded-full px-6 text-sm font-semibold transition-colors ${
                waiting
                  ? "cursor-not-allowed bg-slate-800 text-slate-500"
                  : "bg-sky-500 text-sky-950 hover:bg-sky-400"
              }`}
            >
              {btnLabel}
            </button>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
