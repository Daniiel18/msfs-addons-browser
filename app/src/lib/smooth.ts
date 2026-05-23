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

/**
 * (v3.1.0) Filtra coordenadas inválidas o claramente corruptas que
 * causarían los artefactos visuales que el usuario reportó ("líneas
 * raras cruzando el mapa de lado a lado"):
 *
 *   · `(0, 0)` ó coords cercanas al ecuador-meridiano-cero — el
 *     watcher de SimConnect ocasionalmente reportaba (0,0) durante el
 *     splash/menu y antes guardábamos esos puntos en flight_log_track.
 *   · NaN, Infinity, o números fuera del rango válido.
 *   · Saltos enormes (>500 nm en un solo tick de 10s = >180,000 kt,
 *     físicamente imposible) que sugieren teleport o coord corrupta.
 *
 * Esta función se aplica ANTES del smoothing para que la spline no
 * extienda los puntos basura.
 */
export function sanitizeTrackCoords(points: LngLat[]): LngLat[] {
  const result: LngLat[] = [];
  let prev: LngLat | null = null;
  for (const [lon, lat] of points) {
    // 1. Validar finite + en rango.
    if (!Number.isFinite(lon) || !Number.isFinite(lat)) continue;
    if (lon < -180 || lon > 180) continue;
    if (lat < -90 || lat > 90) continue;
    // 2. Descartar (0, 0) o muy cerca (Atlántico ecuatorial — sin
    //    aeropuertos reales en una circunferencia de 0.05° del cero).
    if (Math.abs(lon) < 0.05 && Math.abs(lat) < 0.05) continue;
    // 3. Saltos imposibles entre samples consecutivos. 5° de delta
    //    en un sample (~10s) son ~300 nm — más del Mach 100 si fuese
    //    real. Aceptamos hasta 5° por si el muestreo se saltó
    //    timestamps (durante pausa larga, etc).
    if (prev) {
      const dlon = Math.abs(lon - prev[0]);
      const dlat = Math.abs(lat - prev[1]);
      // Atajo dateline: si el delta lon es > 180, lo normalizamos.
      const dlonNorm = dlon > 180 ? 360 - dlon : dlon;
      if (dlonNorm > 5 || dlat > 5) {
        // Saltamos este punto pero NO actualizamos prev — esperamos
        // al siguiente punto razonable. Esto evita crear una línea
        // recta enorme entre dos sub-secuencias separadas.
        continue;
      }
    }
    result.push([lon, lat]);
    prev = [lon, lat];
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
