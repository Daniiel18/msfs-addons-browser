import { useState } from "react";

/**
 * (v4.8.0) Logo de aerolínea por código ICAO.
 *
 * Fuente: CDN de airhex (`content.airhex.com`), que indexa por ICAO de
 * 3 letras — el dato que ya guardamos por vuelo (`airlineIcao`). Si el
 * logo no carga (offline, ICAO desconocido, o el CDN no lo tiene) caemos
 * a un chip elegante con el código/iniciales, así NUNCA se ve roto.
 *
 * Tamaño en px; pedimos el bitmap a 2× para nitidez en pantallas HiDPI.
 */
const airhexUrl = (icao: string, px: number) =>
  `https://content.airhex.com/content/logos/airlines_${icao}_${px}_${px}_s.png?proportions=keep`;

export function AirlineLogo({
  icao,
  name,
  size = 20,
  className = "",
}: {
  icao?: string | null;
  name?: string | null;
  size?: number;
  className?: string;
}) {
  const code = (icao ?? "").toUpperCase().trim();
  const valid = /^[A-Z]{3}$/.test(code);
  const [failed, setFailed] = useState(false);

  if (!valid || failed) {
    const initials =
      code ||
      (name ?? "")
        .replace(/[^A-Za-z]/g, "")
        .slice(0, 2)
        .toUpperCase() ||
      "✈";
    return (
      <span
        title={name ?? code ?? undefined}
        style={{
          width: size,
          height: size,
          fontSize: Math.max(7, Math.round(size * 0.38)),
        }}
        className={`inline-flex shrink-0 items-center justify-center rounded bg-slate-700/60 font-mono font-semibold leading-none text-slate-300 ring-1 ring-slate-600/50 ${className}`}
      >
        {initials.slice(0, 3)}
      </span>
    );
  }

  const px = Math.round(size * 2);
  return (
    <img
      src={airhexUrl(code, px)}
      alt={name ?? code}
      title={name ?? code}
      width={size}
      height={size}
      loading="lazy"
      onError={() => setFailed(true)}
      style={{ width: size, height: size, objectFit: "contain" }}
      className={`inline-block shrink-0 rounded bg-white/90 ${className}`}
    />
  );
}
