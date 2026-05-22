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
