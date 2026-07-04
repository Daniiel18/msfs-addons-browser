import type { FlightLogEntry } from "./types";
import { api } from "./tauri";

/**
 * (v6.2.31 / R8) Exporta el FlightBook a un archivo HTML AUTOCONTENIDO
 * y de SOLO LECTURA: se abre en cualquier navegador sin conexión ni
 * SimFleet instalado. Incluye CSS inline, un mapa de rutas SVG en
 * proyección equirectangular (sin depender de tiles online) y la tabla
 * de vuelos. Pensado para compartir tu bitácora con quien quieras.
 */

export interface FlightBookExportLabels {
  title: string;
  subtitle: string;
  flights: string;
  hours: string;
  distance: string;
  airports: string;
  colDate: string;
  colRoute: string;
  colAircraft: string;
  colDistance: string;
  colDuration: string;
  colLanding: string;
  footer: string;
  routeMap: string;
}

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function proj(lat: number, lon: number, W: number, H: number): [number, number] {
  const x = ((lon + 180) / 360) * W;
  const y = ((90 - lat) / 180) * H;
  return [x, y];
}

function buildRouteMap(entries: FlightLogEntry[]): string {
  const W = 1000;
  const H = 500;
  const lines: string[] = [];
  const dots = new Map<string, [number, number]>();
  for (const e of entries) {
    if (
      e.originLat == null ||
      e.originLon == null ||
      e.destinationLat == null ||
      e.destinationLon == null
    )
      continue;
    const [x1, y1] = proj(e.originLat, e.originLon, W, H);
    const [x2, y2] = proj(e.destinationLat, e.destinationLon, W, H);
    // Curva ligera (control point desplazado) para sensación de arco.
    const mx = (x1 + x2) / 2;
    const my = (y1 + y2) / 2 - Math.min(60, Math.hypot(x2 - x1, y2 - y1) * 0.15);
    lines.push(
      `<path d="M${x1.toFixed(1)},${y1.toFixed(1)} Q${mx.toFixed(1)},${my.toFixed(1)} ${x2.toFixed(1)},${y2.toFixed(1)}" fill="none" stroke="#38bdf8" stroke-width="1" stroke-opacity="0.5"/>`,
    );
    dots.set(`${x1.toFixed(1)},${y1.toFixed(1)}`, [x1, y1]);
    dots.set(`${x2.toFixed(1)},${y2.toFixed(1)}`, [x2, y2]);
  }
  const dotEls = [...dots.values()]
    .map(([x, y]) => `<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="2.5" fill="#f8fafc"/>`)
    .join("");
  // Graticule suave cada 30° lon / 30° lat.
  const grid: string[] = [];
  for (let lon = -180; lon <= 180; lon += 30) {
    const x = ((lon + 180) / 360) * W;
    grid.push(`<line x1="${x}" y1="0" x2="${x}" y2="${H}" stroke="#1e293b" stroke-width="0.5"/>`);
  }
  for (let lat = -90; lat <= 90; lat += 30) {
    const y = ((90 - lat) / 180) * H;
    grid.push(`<line x1="0" y1="${y}" x2="${W}" y2="${y}" stroke="#1e293b" stroke-width="0.5"/>`);
  }
  return `<svg viewBox="0 0 ${W} ${H}" class="map" xmlns="http://www.w3.org/2000/svg"><rect width="${W}" height="${H}" fill="#0b1220"/>${grid.join("")}${lines.join("")}${dotEls}</svg>`;
}

export function buildFlightBookHtml(
  entries: FlightLogEntry[],
  pilotName: string,
  labels: FlightBookExportLabels,
  locale: string,
): string {
  const completed = entries.filter((e) => !e.status || e.status === "completed");
  const totalHours =
    completed.reduce((s, e) => s + (e.flightTimeS ?? 0), 0) / 3600;
  const totalDist = completed.reduce((s, e) => s + (e.distanceNm ?? 0), 0);
  const airports = new Set<string>();
  for (const e of completed) {
    if (e.originIcao) airports.add(e.originIcao.toUpperCase());
    if (e.destinationIcao) airports.add(e.destinationIcao.toUpperCase());
  }
  const nf = (n: number) => Math.round(n).toLocaleString(locale);
  const fmtDate = (iso: string) => {
    const d = new Date(iso);
    return isNaN(d.getTime())
      ? ""
      : d.toLocaleDateString(locale, { day: "2-digit", month: "short", year: "numeric" });
  };
  const fmtDur = (s: number | null) =>
    s == null ? "—" : `${Math.floor(s / 3600)}h ${Math.round((s % 3600) / 60)}m`;

  const rows = [...completed]
    .sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""))
    .map((e) => {
      const route = `${esc(e.originIcao ?? "—")} → ${esc(e.destinationIcao ?? "—")}`;
      const ac = esc(e.aircraftTitle || e.aircraftModel || e.aircraftAtcType || "—");
      const dist = e.distanceNm != null ? `${nf(e.distanceNm)} NM` : "—";
      const grade = e.scoreGrade ?? "—";
      return `<tr><td>${esc(fmtDate(e.startedAt))}</td><td class="route">${route}</td><td>${ac}</td><td class="num">${dist}</td><td class="num">${fmtDur(e.flightTimeS)}</td><td class="grade g-${esc((grade || "x").toLowerCase())}">${esc(grade)}</td></tr>`;
    })
    .join("");

  const routeMap = buildRouteMap(completed);

  return `<!DOCTYPE html>
<html lang="${locale.startsWith("es") ? "es" : "en"}">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>${esc(labels.title)}${pilotName ? " — " + esc(pilotName) : ""}</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; background:#0b1220; color:#e2e8f0; font-family:'Segoe UI',Roboto,Helvetica,Arial,sans-serif; }
  .wrap { max-width:1040px; margin:0 auto; padding:32px 20px 56px; }
  header h1 { margin:0; font-size:28px; font-weight:800; }
  header p { margin:4px 0 0; color:#94a3b8; font-size:14px; }
  .stats { display:grid; grid-template-columns:repeat(4,1fr); gap:12px; margin:24px 0; }
  .stat { background:#0f172a; border:1px solid #1e293b; border-radius:14px; padding:16px; text-align:center; }
  .stat .v { font-size:30px; font-weight:800; color:#f8fafc; }
  .stat .l { font-size:12px; color:#94a3b8; text-transform:uppercase; letter-spacing:1px; margin-top:4px; }
  .section-title { font-size:12px; text-transform:uppercase; letter-spacing:1px; color:#64748b; margin:28px 0 10px; }
  .map { width:100%; height:auto; border:1px solid #1e293b; border-radius:14px; display:block; }
  table { width:100%; border-collapse:collapse; margin-top:8px; font-size:14px; }
  th, td { text-align:left; padding:9px 12px; border-bottom:1px solid #17233b; }
  th { color:#64748b; font-size:11px; text-transform:uppercase; letter-spacing:1px; }
  tr:hover td { background:#0f172a; }
  td.route { font-family:ui-monospace,monospace; color:#cbd5e1; }
  td.num { text-align:right; color:#cbd5e1; white-space:nowrap; }
  td.grade { text-align:center; font-weight:800; }
  .g-a{color:#34d399}.g-b{color:#a3e635}.g-c{color:#fbbf24}.g-d{color:#fb923c}.g-f{color:#f87171}.g-x{color:#64748b}
  footer { margin-top:32px; text-align:center; color:#475569; font-size:12px; }
  @media (max-width:640px){ .stats{grid-template-columns:repeat(2,1fr)} }
</style>
</head>
<body>
<div class="wrap">
  <header>
    <h1>${esc(labels.title)}</h1>
    <p>${pilotName ? esc(pilotName) + " · " : ""}${esc(labels.subtitle)}</p>
  </header>
  <div class="stats">
    <div class="stat"><div class="v">${nf(completed.length)}</div><div class="l">${esc(labels.flights)}</div></div>
    <div class="stat"><div class="v">${nf(totalHours)}</div><div class="l">${esc(labels.hours)}</div></div>
    <div class="stat"><div class="v">${nf(totalDist)}</div><div class="l">${esc(labels.distance)}</div></div>
    <div class="stat"><div class="v">${nf(airports.size)}</div><div class="l">${esc(labels.airports)}</div></div>
  </div>
  <div class="section-title">${esc(labels.routeMap)}</div>
  ${routeMap}
  <div class="section-title">${esc(labels.flights)}</div>
  <table>
    <thead><tr><th>${esc(labels.colDate)}</th><th>${esc(labels.colRoute)}</th><th>${esc(labels.colAircraft)}</th><th style="text-align:right">${esc(labels.colDistance)}</th><th style="text-align:right">${esc(labels.colDuration)}</th><th style="text-align:center">${esc(labels.colLanding)}</th></tr></thead>
    <tbody>${rows}</tbody>
  </table>
  <footer>${esc(labels.footer)}</footer>
</div>
</body>
</html>`;
}

/** Guarda el HTML a disco vía el diálogo nativo + save_binary_file
 *  (codificamos UTF-8 → base64). Devuelve la ruta o null si se canceló. */
export async function saveFlightBookHtml(
  html: string,
  defaultName: string,
): Promise<string | null> {
  const path = await api.pickSavePath(defaultName, [
    { name: "HTML", extensions: ["html"] },
  ]);
  if (!path) return null;
  const b64 = btoa(unescape(encodeURIComponent(html)));
  await api.saveBinaryFile(path, b64);
  return path;
}
