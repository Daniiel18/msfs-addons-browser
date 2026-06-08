import { useState } from "react";
import { icaoToIata } from "../lib/airlineCodes";

/**
 * (v4.9.1) Logo de aerolínea por código ICAO.
 *
 * Guardamos el ICAO de 3 letras por vuelo, pero los CDN de logos
 * gratuitos (avs.io) indexan por IATA de 2 letras. Mapeamos ICAO→IATA
 * con `airlineCodes.ts` y pedimos el bitmap a avs.io.
 *
 * Usamos el endpoint `al_square` (isotipo CUADRADO, PNG transparente)
 * en vez del rectangular: queda limpio en chips y panel, sin recuadro
 * blanco. Los logos transparentes (p.ej. el "vencejo" rojo de China
 * Eastern) se integran en el fondo oscuro; los que traen su propio
 * recuadro de marca (p.ej. SAS azul) se ven como un icono de app.
 *
 * Importante: solo pedimos red cuando TENEMOS un IATA mapeado. Para un
 * código desconocido los CDN devuelven 200 con un placeholder genérico
 * (no dispara onError) — así que en ese caso mostramos directamente un
 * chip con el código, sin red. Si el logo mapeado falla al cargar,
 * también caemos al chip.
 *
 * (airhex quedó descartado: devuelve 403 sin API key.)
 */
const avsUrl = (iata: string, px: number) =>
  `https://pics.avs.io/al_square/${px}/${px}/${iata}.png`;

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
  const iata = icaoToIata(code);
  const [failed, setFailed] = useState(false);

  if (!iata || failed) {
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

  // 2× para nitidez en pantallas HiDPI.
  const px = Math.round(size * 2);
  return (
    <img
      src={avsUrl(iata, px)}
      alt={name ?? code}
      title={name ?? code}
      width={size}
      height={size}
      loading="lazy"
      onError={() => setFailed(true)}
      style={{ width: size, height: size, objectFit: "contain" }}
      className={`inline-block shrink-0 rounded ${className}`}
    />
  );
}
