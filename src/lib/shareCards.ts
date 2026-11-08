import type { FlightLogEntry } from "./types";
import type { WrappedStats } from "./wrapped";

/**
 * (v6.2.30 / R7) Generadores de tarjetas SVG compartibles: una para un
 * vuelo concreto y otra para el "Wrapped" anual. Autocontenidas (sólo
 * vector + texto, fuente del sistema) para poder rasterizarlas a PNG en
 * un canvas sin "tainting". Los textos ya vienen traducidos (labels)
 * para no acoplar el generador al i18n.
 */

export interface BuiltCard {
  svg: string;
  width: number;
  height: number;
  defaultName: string;
}

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

const FONT = "Segoe UI, Roboto, Helvetica, Arial, sans-serif";

// (v7.4.2) Medición real del texto (canvas 2D) para que nada se corte a lo
// bruto: antes los nombres se truncaban con `slice(0, 26)` → "Miami
// International Airpor". Fuera del navegador (tests) se aproxima.
let measureCtx: CanvasRenderingContext2D | null | undefined;
function textWidth(text: string, size: number, weight: number): number {
  if (measureCtx === undefined) {
    measureCtx =
      typeof document !== "undefined"
        ? document.createElement("canvas").getContext("2d")
        : null;
  }
  if (!measureCtx) return text.length * size * 0.56;
  measureCtx.font = `${weight} ${size}px ${FONT}`;
  return measureCtx.measureText(text).width;
}

/** Ajusta `text` a `maxWidth`: primero reduce la fuente hasta `minSize` y,
 *  si aun así no cabe, lo corta en un límite de palabra con "…". */
function fitText(
  text: string,
  maxWidth: number,
  size: number,
  weight: number,
  minSize: number,
): { text: string; size: number } {
  let s = size;
  while (s > minSize && textWidth(text, s, weight) > maxWidth) s -= 1;
  if (textWidth(text, s, weight) <= maxWidth) return { text, size: s };
  const words = text.split(/\s+/);
  let out = "";
  for (const w of words) {
    const next = out ? `${out} ${w}` : w;
    if (textWidth(`${next}…`, s, weight) > maxWidth) break;
    out = next;
  }
  if (!out) {
    // Una sola palabra larguísima: corte por caracteres.
    out = text;
    while (out.length > 1 && textWidth(`${out}…`, s, weight) > maxWidth) out = out.slice(0, -1);
  }
  return { text: `${out}…`, size: s };
}

/** `<text>` con el texto ya ajustado a `maxWidth`. */
function fittedText(
  attrs: string,
  text: string,
  maxWidth: number,
  size: number,
  weight: number,
  minSize: number,
): string {
  const f = fitText(text, maxWidth, size, weight, minSize);
  return `<text ${attrs} font-family="${FONT}" font-size="${f.size}" font-weight="${weight}">${esc(f.text)}</text>`;
}

function gradeColor(grade: string | null): string {
  switch ((grade ?? "").toUpperCase()) {
    case "A":
      return "#34d399";
    case "B":
      return "#a3e635";
    case "C":
      return "#fbbf24";
    case "D":
      return "#fb923c";
    case "F":
      return "#f87171";
    default:
      return "#a1a1a6";
  }
}

export interface FlightCardLabels {
  brand: string;
  distance: string;
  duration: string;
  ceiling: string;
  landing: string;
  aircraft: string;
  tagline: string;
}

export function buildFlightCardSvg(
  e: FlightLogEntry,
  labels: FlightCardLabels,
  locale: string,
): BuiltCard {
  const W = 620;
  const H = 720;
  const orig = e.originIcao ?? "----";
  const dest = e.destinationIcao ?? "----";
  const origName = e.originName ?? "";
  const destName = e.destinationName ?? "";
  const date = e.startedAt
    ? new Date(e.startedAt).toLocaleDateString(locale, {
        day: "2-digit",
        month: "long",
        year: "numeric",
      })
    : "";
  const distance = e.distanceNm != null ? `${Math.round(e.distanceNm)} NM` : "—";
  const dur =
    e.flightTimeS != null
      ? `${Math.floor(e.flightTimeS / 3600)}h ${Math.round((e.flightTimeS % 3600) / 60)}m`
      : "—";
  const ceiling = e.maxAltitudeFt != null ? `${Math.round(e.maxAltitudeFt).toLocaleString(locale)} ft` : "—";
  const ac = e.aircraftTitle || e.aircraftModel || e.aircraftAtcType || "—";
  const airline = e.aircraftAirline || e.airlineIcao || "";
  const fpm = e.landingFpm != null ? `${Math.round(e.landingFpm)} fpm` : "—";
  // Cada ICAO/nombre tiene media tarjeta (menos un hueco central).
  const HALF = (W - 80) / 2 - 12;
  // Mismo tamaño a ambos lados (el menor de los dos ajustes) para que
  // origen y destino se vean parejos; lo que aún no quepa lleva "…".
  const icaoSize = Math.min(fitText(orig, HALF, 92, 800, 56).size, fitText(dest, HALF, 92, 800, 56).size);
  const nameSize = Math.min(fitText(origName, HALF, 17, 400, 14).size, fitText(destName, HALF, 17, 400, 14).size);
  const gcol = gradeColor(e.scoreGrade);
  const grade = e.scoreGrade ?? "—";

  const stat = (x: number, value: string, label: string) => `
    <text x="${x}" y="470" font-family="${FONT}" font-size="30" font-weight="700" fill="#ededf0" text-anchor="middle">${esc(value)}</text>
    <text x="${x}" y="496" font-family="${FONT}" font-size="15" fill="#a1a1a6" text-anchor="middle" letter-spacing="1">${esc(label.toUpperCase())}</text>`;

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#141416"/>
      <stop offset="0.55" stop-color="#1c1c1e"/>
      <stop offset="1" stop-color="#1a1a1c"/>
    </linearGradient>
    <linearGradient id="accent" gradientUnits="userSpaceOnUse" x1="60" y1="0" x2="${W - 60}" y2="0">
      <stop offset="0" stop-color="#38bdf8"/>
      <stop offset="1" stop-color="#818cf8"/>
    </linearGradient>
  </defs>
  <rect width="${W}" height="${H}" fill="url(#bg)"/>
  <rect x="10" y="10" width="${W - 20}" height="${H - 20}" rx="22" fill="none" stroke="#2c2c2e" stroke-width="2"/>

  <text x="40" y="62" font-family="${FONT}" font-size="22" font-weight="800" fill="#e0e0e4">${esc(labels.brand)}</text>
  <text x="${W - 40}" y="62" font-family="${FONT}" font-size="16" fill="#a1a1a6" text-anchor="end">${esc(date)}</text>
  <rect x="40" y="80" width="${W - 80}" height="2" fill="#2c2c2e"/>

  ${fittedText(`x="40" y="205" fill="#f5f5f7"`, orig, HALF, icaoSize, 800, icaoSize)}
  ${fittedText(`x="${W - 40}" y="205" fill="#f5f5f7" text-anchor="end"`, dest, HALF, icaoSize, 800, icaoSize)}
  ${fittedText(`x="40" y="238" fill="#a1a1a6"`, origName, HALF, nameSize, 400, nameSize)}
  ${fittedText(`x="${W - 40}" y="238" fill="#a1a1a6" text-anchor="end"`, destName, HALF, nameSize, 400, nameSize)}

  <line x1="60" y1="300" x2="${W - 60}" y2="300" stroke="url(#accent)" stroke-width="3" stroke-dasharray="2 10" stroke-linecap="round"/>
  <circle cx="60" cy="300" r="7" fill="#38bdf8"/>
  <circle cx="${W - 60}" cy="300" r="7" fill="#818cf8"/>
  <g transform="translate(${W / 2}, 300) rotate(90)">
    <path d="M21 16v-2l-8-5V3.5C13 2.67 12.33 2 11.5 2S10 2.67 10 3.5V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z" fill="#e0e0e4" transform="translate(-16,-16) scale(1.35)"/>
  </g>

  <text x="40" y="360" font-family="${FONT}" font-size="14" fill="#7a7a80" letter-spacing="1">${esc(labels.aircraft.toUpperCase())}</text>
  ${fittedText(`x="40" y="390" fill="#e0e0e4"`, ac, W - 80, 26, 700, 18)}
  ${airline ? fittedText(`x="40" y="416" fill="#a1a1a6"`, airline, W - 80, 16, 400, 13) : ""}

  <rect x="40" y="430" width="${W - 80}" height="90" rx="16" fill="#141416" stroke="#2c2c2e"/>
  ${stat(160, distance, labels.distance)}
  ${stat(310, dur, labels.duration)}
  ${stat(470, ceiling, labels.ceiling)}

  <rect x="40" y="545" width="${W - 80}" height="110" rx="16" fill="#141416" stroke="#2c2c2e"/>
  <text x="66" y="588" font-family="${FONT}" font-size="14" fill="#7a7a80" letter-spacing="1">${esc(labels.landing.toUpperCase())}</text>
  <text x="66" y="628" font-family="${FONT}" font-size="40" font-weight="800" fill="#e0e0e4">${esc(fpm)}</text>
  <circle cx="${W - 100}" cy="600" r="42" fill="none" stroke="${gcol}" stroke-width="5"/>
  <text x="${W - 100}" y="616" font-family="${FONT}" font-size="46" font-weight="800" fill="${gcol}" text-anchor="middle">${esc(grade)}</text>

  <text x="${W / 2}" y="695" font-family="${FONT}" font-size="14" fill="#5a5a5f" text-anchor="middle">${esc(labels.tagline)}</text>
</svg>`;

  const name = `simfleet-${orig}-${dest}.png`.replace(/[^a-zA-Z0-9._-]/g, "");
  return { svg, width: W, height: H, defaultName: name };
}

export interface WrappedLabels {
  title: string; // "Wrapped 2026"
  pilot: string; // pilot display name (may be "")
  flights: string;
  hours: string;
  distance: string;
  airports: string;
  topDestinations: string;
  topAircraft: string;
  bestLanding: string;
  tagline: string;
}

export function buildWrappedSvg(
  s: WrappedStats,
  labels: WrappedLabels,
  locale: string,
): BuiltCard {
  const W = 620;
  const H = 820;
  const nf = (n: number) => Math.round(n).toLocaleString(locale);

  const bigStat = (
    x: number,
    y: number,
    value: string,
    label: string,
  ) => `
    <text x="${x}" y="${y}" font-family="${FONT}" font-size="52" font-weight="800" fill="#f5f5f7" text-anchor="middle">${esc(value)}</text>
    <text x="${x}" y="${y + 28}" font-family="${FONT}" font-size="15" fill="#c4b5fd" text-anchor="middle" letter-spacing="1">${esc(label.toUpperCase())}</text>`;

  const destLines = s.topDestinations
    .map(
      (d, i) => `
    <text x="42" y="${560 + i * 34}" font-family="${FONT}" font-size="20" fill="#e0e0e4"><tspan fill="#a78bfa" font-weight="700">${i + 1}.</tspan> ${esc(fitText(d.label, W - 150, 20, 400, 20).text)}</text>
    <text x="${W - 42}" y="${560 + i * 34}" font-family="${FONT}" font-size="20" fill="#a1a1a6" text-anchor="end">${d.count}</text>`,
    )
    .join("");

  const topAc = s.topAircraft[0];
  const bestLanding =
    s.bestLandingFpm != null ? `${Math.round(s.bestLandingFpm)} fpm` : "—";

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <defs>
    <linearGradient id="wbg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#1e1b4b"/>
      <stop offset="0.5" stop-color="#1c1c1e"/>
      <stop offset="1" stop-color="#3b0764"/>
    </linearGradient>
  </defs>
  <rect width="${W}" height="${H}" fill="url(#wbg)"/>
  <rect x="10" y="10" width="${W - 20}" height="${H - 20}" rx="22" fill="none" stroke="#4c1d95" stroke-width="2"/>

  <text x="40" y="70" font-family="${FONT}" font-size="18" font-weight="800" fill="#c4b5fd" letter-spacing="2">SIMFLEET</text>
  <text x="40" y="140" font-family="${FONT}" font-size="64" font-weight="800" fill="#f5f5f7">${esc(labels.title)}</text>
  ${labels.pilot ? `<text x="40" y="176" font-family="${FONT}" font-size="20" fill="#a78bfa">${esc(labels.pilot)}</text>` : ""}

  <rect x="40" y="210" width="${W - 80}" height="180" rx="18" fill="#150f33" stroke="#4c1d95"/>
  ${bigStat(200, 285, nf(s.flights), labels.flights)}
  ${bigStat(430, 285, `${nf(s.hours)}h`, labels.hours)}
  ${bigStat(200, 360, nf(s.distanceNm), labels.distance)}
  ${bigStat(430, 360, nf(s.airportsVisited), labels.airports)}

  <text x="42" y="450" font-family="${FONT}" font-size="15" fill="#c4b5fd" letter-spacing="1">${esc(labels.topAircraft.toUpperCase())}</text>
  ${fittedText(`x="42" y="486" fill="#ededf0"`, topAc?.label ?? "—", W - 84, 30, 700, 20)}

  <text x="42" y="536" font-family="${FONT}" font-size="15" fill="#c4b5fd" letter-spacing="1">${esc(labels.topDestinations.toUpperCase())}</text>
  ${destLines}

  <rect x="40" y="712" width="${W - 80}" height="60" rx="14" fill="#150f33" stroke="#4c1d95"/>
  <text x="62" y="740" font-family="${FONT}" font-size="14" fill="#c4b5fd" letter-spacing="1">${esc(labels.bestLanding.toUpperCase())}</text>
  <text x="62" y="762" font-family="${FONT}" font-size="22" font-weight="800" fill="#34d399">${esc(bestLanding)}</text>

  <text x="${W / 2}" y="800" font-family="${FONT}" font-size="14" fill="#7c6bae" text-anchor="middle">${esc(labels.tagline)}</text>
</svg>`;

  return {
    svg,
    width: W,
    height: H,
    defaultName: `simfleet-wrapped-${s.year}.png`,
  };
}
