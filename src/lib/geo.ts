/** Punto del mapa: [lon, lat]. */
export type Pt = [number, number];

const EARTH_NM = 3440.065;
const RAD = Math.PI / 180;

/** Distancia great-circle en NM. */
export function gcNm(a: Pt, b: Pt): number {
  const dLat = (b[1] - a[1]) * RAD;
  const dLon = (b[0] - a[0]) * RAD;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a[1] * RAD) * Math.cos(b[1] * RAD) * Math.sin(dLon / 2) ** 2;
  return 2 * EARTH_NM * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** Arco de círculo máximo entre dos puntos (para un plan sin navlog). */
export function greatCircle(a: Pt, b: Pt, n = 64): Pt[] {
  const r = RAD;
  const [l1, p1, l2, p2] = [a[0] * r, a[1] * r, b[0] * r, b[1] * r];
  const d = 2 * Math.asin(Math.sqrt(Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2));
  if (d === 0) return [a, b];
  const out: Pt[] = [];
  for (let i = 0; i <= n; i++) {
    const f = i / n;
    const A = Math.sin((1 - f) * d) / Math.sin(d);
    const B = Math.sin(f * d) / Math.sin(d);
    const x = A * Math.cos(p1) * Math.cos(l1) + B * Math.cos(p2) * Math.cos(l2);
    const y = A * Math.cos(p1) * Math.sin(l1) + B * Math.cos(p2) * Math.sin(l2);
    const z = A * Math.sin(p1) + B * Math.sin(p2);
    out.push([Math.atan2(y, x) / r, Math.atan2(z, Math.sqrt(x * x + y * y)) / r]);
  }
  return out;
}

/** Longitudes continuas (sin saltos de ±360) para que la línea no cruce el
 *  mapa entero al pasar el antimeridiano. */
export function unwrap(pts: Pt[]): Pt[] {
  const out: Pt[] = [];
  for (const p of pts) {
    const prev = out[out.length - 1];
    if (!prev) {
      out.push(p);
      continue;
    }
    let lon = p[0];
    while (lon - prev[0] > 180) lon -= 360;
    while (lon - prev[0] < -180) lon += 360;
    out.push([lon, p[1]]);
  }
  return out;
}

function bearing(a: Pt, b: Pt): number {
  const dLon = (b[0] - a[0]) * RAD;
  const y = Math.sin(dLon) * Math.cos(b[1] * RAD);
  const x =
    Math.cos(a[1] * RAD) * Math.sin(b[1] * RAD) - Math.sin(a[1] * RAD) * Math.cos(b[1] * RAD) * Math.cos(dLon);
  return Math.atan2(y, x);
}

/** (06/11) Distancia en NM de un punto a un tramo. Va sobre la esfera, así que
 *  el antimeridiano no es un caso aparte. */
function distanceToLeg(p: Pt, a: Pt, b: Pt): number {
  const toP = gcNm(a, p);
  const leg = gcNm(a, b);
  if (leg === 0) return toP;
  const angle = bearing(a, p) - bearing(a, b);
  if (Math.cos(angle) <= 0) return toP;
  const cross = Math.asin(Math.sin(toP / EARTH_NM) * Math.sin(angle));
  const along = Math.acos(Math.min(1, Math.cos(toP / EARTH_NM) / Math.cos(cross))) * EARTH_NM;
  return along > leg ? gcNm(b, p) : Math.abs(cross) * EARTH_NM;
}

/** (06/11) Distancia en NM de un punto a la ruta. */
export function distanceToRoute(p: Pt, route: Pt[]): number {
  if (route.length === 1) return gcNm(p, route[0]);
  let best = Infinity;
  for (let i = 1; i < route.length; i++) best = Math.min(best, distanceToLeg(p, route[i - 1], route[i]));
  return best;
}

/** (06/11) Ruta de un plan: su navlog o, sin él, el círculo máximo. */
export function planRoute(plan: {
  routeFixes?: { lat: number; lon: number }[];
  originLat: number;
  originLon: number;
  destinationLat: number;
  destinationLon: number;
}): Pt[] {
  const fixes = (plan.routeFixes ?? []).map((f): Pt => [f.lon, f.lat]);
  return fixes.length >= 2
    ? fixes
    : greatCircle([plan.originLon, plan.originLat], [plan.destinationLon, plan.destinationLat]);
}
