/**
 * Genera un polyline de N segmentos que aproxima el arco del círculo
 * máximo entre dos puntos de la Tierra. Usar esto en MapLibre hace
 * que las líneas SimBrief/SimConnect se vean curvadas como en una
 * proyección de globo terráqueo (en vez de líneas rectas Mercator
 * que son visualmente "más cortas" pero geográficamente erróneas).
 *
 * Algoritmo: SLERP (spherical linear interpolation) sobre los vectores
 * unitarios cartesianos de cada extremo. Para cada fracción `t` de la
 * distancia angular `δ`, calculamos el punto intermedio en la esfera
 * y lo proyectamos de vuelta a (lon, lat).
 *
 * Antimeridiano: si la línea cruza el meridiano de cambio de fecha
 * (Pacífico, ej. NRT → LAX), el resultado oscila entre +180 y -180
 * y MapLibre lo dibuja "saltando" por el mapa. Para evitarlo,
 * detectamos saltos > 180° entre puntos consecutivos y dividimos el
 * polyline en MultiLineString. La función devuelve `[][]` — una
 * lista de polylines, cada una continua en longitud.
 */
export function greatCircleLine(
  lon1: number,
  lat1: number,
  lon2: number,
  lat2: number,
  segments = 96,
): [number, number][][] {
  const φ1 = (lat1 * Math.PI) / 180;
  const φ2 = (lat2 * Math.PI) / 180;
  const λ1 = (lon1 * Math.PI) / 180;
  const λ2 = (lon2 * Math.PI) / 180;

  const Δφ = φ2 - φ1;
  const Δλ = λ2 - λ1;
  const a =
    Math.sin(Δφ / 2) ** 2 +
    Math.cos(φ1) * Math.cos(φ2) * Math.sin(Δλ / 2) ** 2;
  const δ = 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));

  // Punto único — no hay arco que dibujar.
  if (δ === 0 || !isFinite(δ)) {
    return [[[lon1, lat1], [lon2, lat2]]];
  }

  const points: [number, number][] = [];
  for (let i = 0; i <= segments; i++) {
    const f = i / segments;
    const A = Math.sin((1 - f) * δ) / Math.sin(δ);
    const B = Math.sin(f * δ) / Math.sin(δ);
    const x = A * Math.cos(φ1) * Math.cos(λ1) + B * Math.cos(φ2) * Math.cos(λ2);
    const y = A * Math.cos(φ1) * Math.sin(λ1) + B * Math.cos(φ2) * Math.sin(λ2);
    const z = A * Math.sin(φ1) + B * Math.sin(φ2);
    const lat = Math.atan2(z, Math.sqrt(x * x + y * y));
    let lon = Math.atan2(y, x);
    points.push([(lon * 180) / Math.PI, (lat * 180) / Math.PI]);
  }

  // Detectar saltos > 180° y partir el polyline ahí.
  const segments_out: [number, number][][] = [];
  let current: [number, number][] = [points[0]];
  for (let i = 1; i < points.length; i++) {
    const dlon = Math.abs(points[i][0] - points[i - 1][0]);
    if (dlon > 180) {
      segments_out.push(current);
      current = [points[i]];
    } else {
      current.push(points[i]);
    }
  }
  if (current.length > 0) segments_out.push(current);
  return segments_out;
}
