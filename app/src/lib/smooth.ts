/**
 * (v2.2.0) Suavizado de polylines con Catmull-Rom spline.
 *
 * El watcher de SimConnect samplea posición cada 10 s. A 250 kt eso
 * son ~1.3 km entre cada punto, lo cual al renderizar como
 * LineString muestra giros muy poligonales (especialmente en
 * approaches y patrones cerrados). Aplicar una spline que pase por
 * los puntos crudos da un trazo fluido sin alterar la trayectoria
 * real.
 *
 * Catmull-Rom es ideal aquí porque:
 *   · Pasa EXACTAMENTE por cada punto de entrada (a diferencia de
 *     Bézier), así que no falseamos la ruta.
 *   · La tensión es continua → curvas naturales.
 *   · Coste lineal O(n) con `n` puntos, despreciable.
 *
 * Implementación: para cada segmento entre P[i] y P[i+1] interpolamos
 * `segments` subdivisiones usando los puntos vecinos P[i-1] y P[i+2]
 * como control points. En los extremos duplicamos el primer/último
 * punto para que la spline arranque/termine donde toca.
 *
 * Trabajamos en coordenadas lat/lon DIRECTAMENTE — es una aproximación
 * (no proyectamos a 3D para hacer slerp) pero a distancias de muestreo
 * cortas (<2 km) el error es invisible en el mapa. Para vuelos
 * trans-oceánicos donde los segmentos son largos, las great-circles
 * de `flightLogGeojson` siguen usando `greatCircleLine` que sí
 * proyecta a 3D.
 */

export type LngLat = [number, number];

/** (v3.4.0) Punto con timestamp opcional — permite a sanitize()
 *  detectar gaps temporales (sim pausado, reconexión SimConnect) que
 *  visualmente generan líneas rectas largas aún cuando el delta
 *  lat/lon es chico. */
export interface LngLatTs {
  lon: number;
  lat: number;
  /** ISO timestamp del muestreo. Si no está, sólo se validan
   *  thresholds geométricos. */
  ts?: string;
}

/** Threshold de salto geométrico permitido entre samples consecutivos
 *  (en grados, máximo de |dlat| y |dlon-normalized-para-dateline|).
 *
 *  (v3.5.0 F3) Bumpeado a 3.0° — el threshold anterior de 1.5° era
 *  demasiado estricto para imports VAS-ACARS con tailwind fuerte en
 *  cruise (GS 600+ kt × 5s = 50 nm = 0.83°, pero gaps de sampling
 *  pueden hacer que un par de samples consecutivos vayan a 15-20s →
 *  150 nm = 2.5°). El usuario reportaba "la ruta falla en ciertos
 *  puntos" — eran samples legítimos siendo dropped por el sanitizer.
 *
 *  3.0° = ~180 nm. Cubre cualquier jet civil + tailwind + gaps cortos
 *  de sampling, pero descarta teleports/slew (que típicamente saltan
 *  >500 nm en un solo sample).
 *
 *  Antes (v3.4.0–v3.5.0 F2): 1.5° → caía bajo cruise con tailwind. */
const MAX_DEG_PER_SAMPLE = 3.0;

/** Threshold temporal entre samples (segundos). Si dos puntos están
 *  separados >`MAX_TS_GAP_SECONDS`, asumimos pausa/reconexión y
 *  cortamos la traza (no dibujamos línea uniendo los segmentos).
 *  El sampling normal es cada 10s — un gap >120s significa que el
 *  usuario pausó MSFS o SimConnect se desconectó. */
const MAX_TS_GAP_SECONDS = 120;

/**
 * (v3.1.0 → v3.4.0) Filtra coordenadas inválidas o claramente
 * corruptas que causarían las "líneas raras cruzando el mapa" que
 * el usuario reportó:
 *
 *   · `(0, 0)` o coords cercanas al ecuador-meridiano-cero — el
 *     watcher de SimConnect ocasionalmente reportaba (0,0) durante el
 *     splash/menu y antes guardábamos esos puntos en flight_log_track.
 *   · NaN, Infinity, o números fuera del rango válido.
 *   · Saltos > `MAX_DEG_PER_SAMPLE` (1.5°) entre samples consecutivos
 *     — descarta teleports de slew, "Recoloca avión", o resúmenes
 *     tras crash de la sesión.
 *   · (v3.4.0) Si se proporcionan timestamps, gap temporal
 *     > `MAX_TS_GAP_SECONDS` (120s) corta la traza — el segmento
 *     posterior se renderiza como una **traza separada**, no como
 *     línea recta uniéndolos.
 *
 * Esta función se aplica ANTES del smoothing.
 */
export function sanitizeTrackCoords(points: LngLat[]): LngLat[] {
  return sanitizeTrackCoordsWithTs(
    points.map(([lon, lat]) => ({ lon, lat })),
  ).map((p) => [p.lon, p.lat] as LngLat);
}

/** Variante con timestamps — corta la traza en gaps temporales reales
 *  (no sólo geométricos). Devuelve el mismo shape, omitiendo los
 *  puntos descartados. Los gaps NO se "saltan" — se conservan como
 *  cortes (el render decide si emitir múltiples LineStrings o
 *  perforar con un NaN coordinate, según el caller). */
export function sanitizeTrackCoordsWithTs(
  points: LngLatTs[],
): LngLatTs[] {
  const result: LngLatTs[] = [];
  let prev: { lon: number; lat: number; epoch: number | null } | null = null;
  for (const p of points) {
    // 1. Validar finite + en rango.
    if (!Number.isFinite(p.lon) || !Number.isFinite(p.lat)) continue;
    if (p.lon < -180 || p.lon > 180) continue;
    if (p.lat < -90 || p.lat > 90) continue;
    // 2. Descartar (0, 0) o muy cerca (Atlántico ecuatorial — sin
    //    aeropuertos reales en una circunferencia de 0.05° del cero).
    if (Math.abs(p.lon) < 0.05 && Math.abs(p.lat) < 0.05) continue;
    // 3. Saltos imposibles entre samples consecutivos.
    const epoch = p.ts ? Date.parse(p.ts) : NaN;
    const epochOk = Number.isFinite(epoch);
    if (prev) {
      const dlon = Math.abs(p.lon - prev.lon);
      const dlat = Math.abs(p.lat - prev.lat);
      // Atajo dateline: si el delta lon es > 180, lo normalizamos.
      const dlonNorm = dlon > 180 ? 360 - dlon : dlon;
      if (dlonNorm > MAX_DEG_PER_SAMPLE || dlat > MAX_DEG_PER_SAMPLE) {
        // Saltamos este punto. NO actualizamos `prev` — esperamos al
        // siguiente sample para ver si fue un outlier puntual o si
        // realmente cambió la traza (teleport).
        continue;
      }
      // 4. Gap temporal — sólo si AMBOS puntos tienen ts.
      if (epochOk && prev.epoch != null) {
        const gapSec = (epoch - prev.epoch) / 1000;
        if (gapSec > MAX_TS_GAP_SECONDS) {
          // Sim pausado / reconexión SimConnect. NO uses los puntos
          // anteriores como contexto — empieza una "subsecuencia" nueva
          // reseteando prev. El resultado: el render verá un salto que
          // puede cortar la línea (si quiere) o dejarla recta con
          // smoothing leve (mejor que la línea poligonal larga).
          prev = { lon: p.lon, lat: p.lat, epoch };
          result.push(p);
          continue;
        }
      }
    }
    result.push(p);
    prev = { lon: p.lon, lat: p.lat, epoch: epochOk ? epoch : null };
  }
  return result;
}

/**
 * Suaviza una lista de `points` lat/lon con una spline Catmull-Rom.
 * `segmentsPerStep` controla cuántos puntos intermedios se generan
 * entre cada par de puntos de entrada. Default 8 — buen balance entre
 * suavidad y peso del GeoJSON.
 */
export function smoothCatmullRom(
  points: LngLat[],
  segmentsPerStep = 8,
): LngLat[] {
  if (points.length < 3) return points.slice();
  const result: LngLat[] = [];
  // Duplicamos extremos para mantener la curva pasando por start/end.
  const extended: LngLat[] = [points[0], ...points, points[points.length - 1]];
  for (let i = 1; i < extended.length - 2; i++) {
    const p0 = extended[i - 1];
    const p1 = extended[i];
    const p2 = extended[i + 1];
    const p3 = extended[i + 2];
    for (let t = 0; t < segmentsPerStep; t++) {
      const u = t / segmentsPerStep;
      result.push([
        catmullRomComponent(p0[0], p1[0], p2[0], p3[0], u),
        catmullRomComponent(p0[1], p1[1], p2[1], p3[1], u),
      ]);
    }
  }
  // Asegurar el último punto exacto.
  result.push(points[points.length - 1]);
  return result;
}

/**
 * Componente escalar de la spline Catmull-Rom uniforme:
 *
 *   q(t) = 0.5 * [2*P1 + (-P0+P2)*t + (2*P0-5*P1+4*P2-P3)*t² + (-P0+3*P1-3*P2+P3)*t³]
 *
 * con `tension = 0.5` (la variante estándar Catmull-Rom).
 */
function catmullRomComponent(
  p0: number,
  p1: number,
  p2: number,
  p3: number,
  t: number,
): number {
  const t2 = t * t;
  const t3 = t2 * t;
  return (
    0.5 *
    (2 * p1 +
      (-p0 + p2) * t +
      (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 +
      (-p0 + 3 * p1 - 3 * p2 + p3) * t3)
  );
}
