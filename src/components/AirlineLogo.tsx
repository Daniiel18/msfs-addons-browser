import { useState } from "react";
import { resolveIata } from "../lib/airlineCodes";

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

// (v6.1) Respaldo: avs.io NO tiene logos de CARGA (DHL, FedEx, etc. dan 404).
// daisycon sí los sirve por IATA. Lo usamos solo cuando avs.io falla y ya
// tenemos un IATA mapeado (aerolínea conocida) — así no metemos su placeholder
// genérico para códigos desconocidos (esos van directo al chip).
const daisyconUrl = (iata: string, px: number) =>
  `https://images.daisycon.io/airline/?width=${px}&height=${px}&color=ffffff&iata=${iata}`;

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
  // ICAO autoritativo primero; si no mapea, resolvemos por el nombre
  // (vuelos importados sin callsign solo traen el nombre/título).
  const iata = resolveIata(code, name);
  // Etapa de carga del logo: avs.io → daisycon (carga) → chip.
  const [stage, setStage] = useState<"avs" | "daisycon" | "chip">("avs");

  if (!iata || stage === "chip") {
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
  const src = stage === "avs" ? avsUrl(iata, px) : daisyconUrl(iata, px);
  return (
    <img
      // `key` fuerza recrear el <img> al cambiar de CDN (re-dispara la carga).
      key={stage}
      src={src}
      alt={name ?? code}
      title={name ?? code}
      width={size}
      height={size}
      loading="lazy"
      onError={() => setStage((s) => (s === "avs" ? "daisycon" : "chip"))}
      style={{ width: size, height: size, objectFit: "contain" }}
      className={`inline-block shrink-0 rounded ${className}`}
    />
  );
}
