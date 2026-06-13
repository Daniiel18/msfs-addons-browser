import { Cog, HelpCircle, Music, Palette, Plane } from "lucide-react";
import type { DerivedType } from "../lib/packageType";

/**
 * (v4.26.0) Arte de respaldo para addons SIN thumbnail.
 *
 * Antes esos cards mostraban un fondo vacío con un iconito gris — el
 * usuario reportó que "no me gusta que se vea así". Ahora pintamos un
 * arte tipado: gradiente con el acento de la categoría, el icono como
 * marca de agua gigante y las iniciales del título — cada card sin
 * imagen sigue viéndose intencional y distinguible.
 */
export function AddonFallbackArt({
  derived,
  title,
}: {
  derived: DerivedType;
  title: string;
}) {
  const tone = toneFor(derived);
  const initials = initialsFor(title);
  return (
    <div
      className={`relative flex h-full w-full items-center justify-center overflow-hidden bg-gradient-to-br ${tone.bg}`}
    >
      {/* Marca de agua: icono gigante sangrado a la derecha. */}
      <div
        className={`pointer-events-none absolute -bottom-4 -right-4 ${tone.icon} opacity-[0.14]`}
      >
        {iconFor(derived, "h-24 w-24")}
      </div>
      {/* Iniciales del addon — identidad rápida sin foto. */}
      <span
        className={`select-none text-3xl font-black tracking-wider ${tone.text} opacity-70`}
      >
        {initials}
      </span>
    </div>
  );
}

function initialsFor(title: string): string {
  const words = title
    .split(/[\s\-_·.]+/)
    .map((w) => w.trim())
    .filter((w) => w.length > 0);
  if (words.length === 0) return "?";
  // Códigos tipo "A350" / "737-800" ya son cortos y reconocibles.
  if (words[0].length <= 5 && /\d/.test(words[0])) {
    return words[0].toUpperCase();
  }
  return words
    .slice(0, 3)
    .map((w) => w.charAt(0).toUpperCase())
    .join("");
}

function toneFor(t: DerivedType): { bg: string; icon: string; text: string } {
  switch (t) {
    case "AIRCRAFT":
      return {
        bg: "from-sky-950 via-slate-900 to-slate-950",
        icon: "text-sky-400",
        text: "text-sky-200",
      };
    case "LIVERY":
      return {
        bg: "from-violet-950 via-slate-900 to-slate-950",
        icon: "text-violet-400",
        text: "text-violet-200",
      };
    case "INSTRUMENT":
      return {
        bg: "from-fuchsia-950 via-slate-900 to-slate-950",
        icon: "text-fuchsia-400",
        text: "text-fuchsia-200",
      };
    case "MISC":
      return {
        bg: "from-amber-950 via-slate-900 to-slate-950",
        icon: "text-amber-400",
        text: "text-amber-200",
      };
    default:
      return {
        bg: "from-slate-800 via-slate-900 to-slate-950",
        icon: "text-slate-400",
        text: "text-slate-300",
      };
  }
}

function iconFor(t: DerivedType, className: string): React.ReactNode {
  switch (t) {
    case "AIRCRAFT":
      return <Plane className={className} />;
    case "LIVERY":
      return <Palette className={className} />;
    case "INSTRUMENT":
      return <Cog className={className} />;
    case "MISC":
      return <Music className={className} />;
    default:
      return <HelpCircle className={className} />;
  }
}
