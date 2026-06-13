import type { DerivedType } from "../lib/packageType";

/**
 * (v4.29.0) Arte premium para addons sin thumbnail real.
 *
 * El usuario quiere: "premium oscuro con la silueta del modelo de
 * avión correspondiente". Adiós a iniciales y iconos pequeños — ahora
 * cada card vacía muestra una silueta GIGANTE de avión en bajo
 * contraste sobre un gradiente oscuro, estilo placeholder de
 * marketplace de MSFS. La silueta varía por tipo de avión cuando se
 * detecta (wide-body / narrow-body / GA); fallback a un perfil
 * comercial genérico para todo lo demás. */
export function AddonFallbackArt({
  derived,
  title,
}: {
  derived: DerivedType;
  title: string;
}) {
  const tone = toneFor(derived);
  return (
    <div
      className={`relative flex h-full w-full items-center justify-center overflow-hidden bg-gradient-to-br ${tone.bg}`}
    >
      {/* Resplandor radial sutil — da profundidad al fondo. */}
      <div className={`pointer-events-none absolute inset-0 ${tone.glow}`} />
      {/* Silueta del avión: SVG inline, full-width. Lo hacemos pasar de
          borde a borde para que se sienta una imagen, no un icono. */}
      <AircraftSilhouette
        kind={kindFor(title, derived)}
        className={`relative h-[58%] w-[78%] ${tone.silhouette}`}
      />
    </div>
  );
}

/** Devuelve el tipo visual de silueta basado en el título / categoría. */
function kindFor(title: string, derived: DerivedType): SilhouetteKind {
  const t = title.toLowerCase();
  if (
    /\b(a380|a380x|a350|a330|a340|747|748|777|787|767|jumbo|dreamliner|wide[\s-]?body)\b/.test(t)
  ) {
    return "widebody";
  }
  if (/\b(c1?7[2358]|cessna|piper|mooney|king\s*air|tbm)\b/.test(t)) {
    return "ga";
  }
  if (/\b(crj|atr|q400|dh[c]?-?8|embraer|e1[79]0|e1[79]5)\b/.test(t)) {
    return "regional";
  }
  if (derived === "INSTRUMENT") return "panel";
  if (derived === "MISC") return "soundwave";
  if (derived === "UNKNOWN") return "questionmark";
  return "narrowbody"; // 737/A320 default — el avión más común
}

type SilhouetteKind =
  | "narrowbody"
  | "widebody"
  | "regional"
  | "ga"
  | "panel"
  | "soundwave"
  | "questionmark";

function AircraftSilhouette({
  kind,
  className,
}: {
  kind: SilhouetteKind;
  className: string;
}) {
  const path = PATHS[kind];
  return (
    <svg
      viewBox="0 0 200 80"
      preserveAspectRatio="xMidYMid meet"
      className={className}
      aria-hidden="true"
    >
      <path d={path} fill="currentColor" />
    </svg>
  );
}

const PATHS: Record<SilhouetteKind, string> = {
  // Narrow-body comercial (A320 / 737) — fuselaje + alas en cruz +
  // estabilizadores. Trazo simplificado, idea: silueta vista desde
  // arriba (top-down) que se lee a cualquier tamaño.
  narrowbody:
    "M100 12 L108 18 L114 28 L120 32 L168 38 L120 42 L114 46 L114 54 L150 58 L114 62 L110 66 L106 70 L100 72 L94 70 L90 66 L86 62 L50 58 L86 54 L86 46 L80 42 L32 38 L80 32 L86 28 L92 18 Z",
  // Wide-body (777 / A350 / 747) — alas más amplias, fuselaje grueso.
  widebody:
    "M100 8 L110 16 L118 30 L124 34 L188 40 L124 44 L118 50 L116 58 L160 62 L116 66 L112 70 L108 74 L100 76 L92 74 L88 70 L84 66 L40 62 L84 58 L82 50 L76 44 L12 40 L76 34 L82 30 L90 16 Z",
  // Regional jet / turboprop — fuselaje fino, alas medianas, hélice
  // sugerida con punta redonda.
  regional:
    "M100 16 L106 22 L112 30 L116 32 L162 36 L116 40 L112 44 L110 54 L140 58 L110 62 L108 66 L104 70 L100 72 L96 70 L92 66 L90 62 L60 58 L90 54 L88 44 L84 40 L38 36 L84 32 L88 30 L94 22 Z",
  // GA (Cessna 172): ala alta, fuselaje corto.
  ga: "M100 26 L106 30 L112 34 L116 36 L172 38 L116 40 L112 42 L108 56 L130 60 L108 62 L100 68 L92 62 L70 60 L92 56 L88 42 L84 40 L28 38 L84 36 L88 34 L94 30 Z",
  // Panel / instrumentos: silueta de tablet/EFB.
  panel:
    "M40 18 L160 18 Q170 18 170 28 L170 60 Q170 70 160 70 L40 70 Q30 70 30 60 L30 28 Q30 18 40 18 Z M44 24 L156 24 L156 64 L44 64 Z",
  // Onda de sonido (sound packs).
  soundwave:
    "M30 40 L40 40 L40 48 L52 32 L52 64 L66 26 L66 70 L80 22 L80 74 L100 14 L100 82 L120 22 L120 74 L134 26 L134 70 L148 32 L148 64 L160 40 L170 40",
  // ?: paquetes sin clasificar — ya casi no salen pero por si acaso.
  questionmark:
    "M100 14 Q120 14 130 32 Q130 48 110 52 Q108 56 108 62 L92 62 Q92 50 96 46 Q112 38 112 32 Q112 26 100 26 Q88 26 88 36 L72 36 Q72 14 100 14 Z M92 70 L108 70 L108 84 L92 84 Z",
};

function toneFor(t: DerivedType): {
  bg: string;
  glow: string;
  silhouette: string;
} {
  switch (t) {
    case "AIRCRAFT":
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        glow: "bg-[radial-gradient(circle_at_30%_30%,rgba(56,189,248,0.18),transparent_60%)]",
        silhouette: "text-sky-200/30",
      };
    case "LIVERY":
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        glow: "bg-[radial-gradient(circle_at_70%_30%,rgba(167,139,250,0.18),transparent_60%)]",
        silhouette: "text-violet-200/35",
      };
    case "INSTRUMENT":
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        glow: "bg-[radial-gradient(circle_at_50%_30%,rgba(232,121,249,0.18),transparent_60%)]",
        silhouette: "text-fuchsia-200/30",
      };
    case "MISC":
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        glow: "bg-[radial-gradient(circle_at_50%_30%,rgba(251,191,36,0.16),transparent_60%)]",
        silhouette: "text-amber-200/30",
      };
    default:
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        glow: "bg-[radial-gradient(circle_at_50%_30%,rgba(148,163,184,0.18),transparent_60%)]",
        silhouette: "text-slate-300/25",
      };
  }
}
