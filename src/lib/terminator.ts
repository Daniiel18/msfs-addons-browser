/**
 * (v4.0.0 — P6 · v7.4.2) Terminator día/noche del RoutesMapView.
 *
 * Computa el polígono GeoJSON de la zona donde el sol está por debajo de
 * `altitudeDeg` (0° = noche geométrica, -6/-12/-18° = crepúsculos) para un
 * instante UTC. Pintado como `fill-layer` sobre el globo.
 *
 * ## Por qué se reescribió (v7.4.2)
 *
 * La versión anterior buscaba, para cada longitud, UNA latitud frontera por
 * bisección entre -90° y +90° y cerraba el polígono contra un polo. Eso sólo
 * vale si la zona oscura toca un polo. Para las bandas de crepúsculo
 * (-12°/-18°) cerca de los equinoccios (|declinación| < |umbral|, p.ej. en
 * octubre) la zona es un casquete que NO toca ningún polo: la bisección no
 * encontraba cambio de signo, devolvía `null` y el código saltaba al polo
 * N o S según la altitud en el ecuador → franjas oscuras de polo a polo que
 * no correspondían con la hora real ("la noche se desincroniza del reloj").
 *
 * ## Matemática
 *
 * Posición del sol con las mismas fórmulas que SunCalc (declinación δ,
 * ascensión recta α, tiempo sidéreo θ₀). Para un punto (φ, λ):
 *
 *   sin(alt) = sin φ · sin δ + cos φ · cos δ · cos H,   H = θ₀ + λ − α
 *
 * Con A = sin δ y B = cos δ · cos H: sin(alt) = R · sin(φ + γ), con
 * R = √(A² + B²) y γ = atan2(B, A). Así, para cada meridiano, la zona
 * `alt < umbral` es UN intervalo de latitudes [lo, hi] (o vacío) que se
 * resuelve de forma exacta con asin — sin bisección.
 *
 * ## Construcción del polígono
 *
 *   · La noche contiene el polo N  → anillo por `lo(λ)` + cierre por +90°.
 *   · La noche contiene el polo S  → anillo por `hi(λ)` + cierre por −90°.
 *   · No toca ningún polo (casquete) → por cada tramo continuo de longitudes
 *     con noche: `hi` de oeste a este y `lo` de vuelta. Si el casquete cruza
 *     el antimeridiano salen dos tramos (MultiPolygon) que comparten borde.
 *
 * Coste: ~361 muestras × 4 bandas con trigonometría directa ≈ <1 ms.
 */

const RAD = Math.PI / 180;
const DEG = 180 / Math.PI;

/** Declinación y longitud subsolar (rad) — fórmulas de SunCalc. */
function sunPosition(date: Date): { dec: number; subLon: number } {
  const d = date.valueOf() / 86_400_000 - 0.5 + 2_440_588 - 2_451_545;
  const M = RAD * (357.5291 + 0.98560028 * d);
  const C = RAD * (1.9148 * Math.sin(M) + 0.02 * Math.sin(2 * M) + 0.0003 * Math.sin(3 * M));
  const L = M + C + RAD * 102.9372 + Math.PI;
  const e = RAD * 23.4397;
  const dec = Math.asin(Math.sin(e) * Math.sin(L));
  const ra = Math.atan2(Math.sin(L) * Math.cos(e), Math.cos(L));
  const theta0 = RAD * (280.16 + 360.9856235 * d);
  // H = θ₀ + λ − α = 0 en el punto subsolar → λ = α − θ₀.
  const subLon = Math.atan2(Math.sin(ra - theta0), Math.cos(ra - theta0));
  return { dec, subLon };
}

/** Intervalo [lo, hi] (grados) de latitudes con alt < umbral en el meridiano
 *  `lonDeg`, o `null` si en ese meridiano no hay noche. */
function nightInterval(
  dec: number,
  subLon: number,
  lonDeg: number,
  sinThreshold: number,
): [number, number] | null {
  const H = lonDeg * RAD - subLon;
  const A = Math.sin(dec);
  const B = Math.cos(dec) * Math.cos(H);
  const R = Math.hypot(A, B);
  if (R < 1e-12) return sinThreshold > 0 ? [-90, 90] : null;
  const k = sinThreshold / R;
  if (k <= -1) return null; // el sol nunca baja del umbral aquí
  if (k >= 1) return [-90, 90];
  const gamma = Math.atan2(B, A);
  const a = Math.asin(k);
  // sin(u) < sin(a)  ⇔  u ∈ (π − a, 2π + a)  (mod 2π),  con u = φ + γ.
  const start = Math.PI - a - gamma;
  const end = 2 * Math.PI + a - gamma;
  let best: [number, number] | null = null;
  for (let n = -2; n <= 2; n++) {
    const lo = Math.max(start + 2 * Math.PI * n, -Math.PI / 2);
    const hi = Math.min(end + 2 * Math.PI * n, Math.PI / 2);
    if (hi > lo && (!best || hi - lo > best[1] - best[0])) best = [lo, hi];
  }
  return best ? [best[0] * DEG, best[1] * DEG] : null;
}

/**
 * Polígono GeoJSON de la zona con el sol por debajo de `altitudeDeg` en el
 * instante `date`.
 *
 * @param date Instante (típicamente `new Date()`); se usa en UTC.
 * @param step Espaciado en longitud entre muestras (°).
 * @param altitudeDeg Umbral de altitud solar (0, -6, -12, -18…).
 */
export function buildTerminatorPolygon(
  date: Date = new Date(),
  step: number = 2,
  altitudeDeg: number = 0,
): GeoJSON.Feature<GeoJSON.Polygon | GeoJSON.MultiPolygon> {
  const { dec, subLon } = sunPosition(date);
  const sinT = Math.sin(altitudeDeg * RAD);
  const lons: number[] = [];
  for (let lon = -180; lon < 180; lon += step) lons.push(lon);
  lons.push(180);
  const samples = lons.map((lon) => ({ lon, iv: nightInterval(dec, subLon, lon, sinT) }));

  // Altitud del sol en cada polo = ±δ → ¿el polo está dentro de la noche?
  const northNight = Math.sin(dec) < sinT;
  const southNight = -Math.sin(dec) < sinT;

  const feature = (
    geometry: GeoJSON.Polygon | GeoJSON.MultiPolygon,
  ): GeoJSON.Feature<GeoJSON.Polygon | GeoJSON.MultiPolygon> => ({
    type: "Feature",
    properties: {},
    geometry,
  });

  if (northNight || southNight) {
    // La noche toca un polo → existe en todos los meridianos; la frontera es
    // el extremo opuesto al polo.
    const ring: [number, number][] = samples.map(({ lon, iv }) => [
      lon,
      iv ? (northNight ? iv[0] : iv[1]) : northNight ? 90 : -90,
    ]);
    const pole = northNight ? 90 : -90;
    ring.push([180, pole], [-180, pole], ring[0]);
    return feature({ type: "Polygon", coordinates: [ring] });
  }

  // Casquete que no toca polos: un polígono por tramo continuo con noche.
  const polygons: [number, number][][][] = [];
  let run: { lon: number; iv: [number, number] }[] = [];
  const flush = () => {
    if (run.length >= 2) {
      const ring: [number, number][] = [
        ...run.map((s): [number, number] => [s.lon, s.iv[1]]),
        ...run
          .slice()
          .reverse()
          .map((s): [number, number] => [s.lon, s.iv[0]]),
      ];
      ring.push(ring[0]);
      polygons.push([ring]);
    }
    run = [];
  };
  for (const s of samples) {
    if (s.iv) run.push({ lon: s.lon, iv: s.iv });
    else flush();
  }
  flush();
  if (polygons.length === 0) {
    return feature({ type: "MultiPolygon", coordinates: [] });
  }
  return feature(
    polygons.length === 1
      ? { type: "Polygon", coordinates: polygons[0] }
      : { type: "MultiPolygon", coordinates: polygons },
  );
}
