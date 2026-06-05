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
 * (v3.23.0) Segmenta el track en sub-trazas, CORTANDO donde hay un
 * salto geométrico imposible (>`MAX_DEG_PER_SAMPLE`) o un gap temporal
 * (>`MAX_TS_GAP_SECONDS`). Devuelve un array de segmentos (cada uno una
 * lista de `[lon,lat]`). El render emite un **MultiLineString** para que
 * las "costuras" entre tramos NO se dibujen como rectas cruzando el mapa.
 *
 * Por qué: algunos tracks viejos/importados (o vuelos re-restaurados)
 * vienen con los puntos de varias sesiones concatenados, o con un punto
 * teleportado. El saneador secuencial sólo descartaba 1 punto por
 * costura, pero el bloque siguiente quedaba unido al anterior por una
 * recta larga. Cortando en cada salto, cada bloque se dibuja por
 * separado (se superponen visualmente = una ruta limpia) y los outliers
 * aislados quedan en segmentos de 1 punto que no se renderizan.
 */
export function segmentTrackCoords(points: LngLatTs[]): LngLat[][] {
  const segments: LngLat[][] = [];
  let cur: LngLat[] = [];
  let prev: { lon: number; lat: number; epoch: number | null } | null = null;
  const flush = () => {
    if (cur.length) {
      segments.push(cur);
      cur = [];
    }
  };
  for (const p of points) {
    if (!Number.isFinite(p.lon) || !Number.isFinite(p.lat)) continue;
    if (p.lon < -180 || p.lon > 180 || p.lat < -90 || p.lat > 90) continue;
    if (Math.abs(p.lon) < 0.05 && Math.abs(p.lat) < 0.05) continue;
    const epoch = p.ts ? Date.parse(p.ts) : NaN;
    const epochOk = Number.isFinite(epoch);
    if (prev) {
      const dlon = Math.abs(p.lon - prev.lon);
      const dlat = Math.abs(p.lat - prev.lat);
      const dlonNorm = dlon > 180 ? 360 - dlon : dlon;
      const geoJump =
        dlonNorm > MAX_DEG_PER_SAMPLE || dlat > MAX_DEG_PER_SAMPLE;
      const tsGap =
        epochOk &&
        prev.epoch != null &&
        (epoch - prev.epoch) / 1000 > MAX_TS_GAP_SECONDS;
      if (geoJump || tsGap) {
        flush(); // corta: el bloque siguiente arranca traza nueva.
      }
    }
    cur.push([p.lon, p.lat]);
    prev = { lon: p.lon, lat: p.lat, epoch: epochOk ? epoch : null };
  }
  flush();
  return segments;
}

/**
 * Suaviza una lista de `points` lat/lon con una spline Catmull-Rom
 * **centrípeta** (α = 0.5).
 *
 * (v4.0.1) Antes usábamos Catmull-Rom UNIFORME, que tiene un defecto
 * conocido: en cambios de dirección bruscos hace *overshoot* y forma
 * lazos/cúspides que se auto-intersectan. Eso producía el "doble
 * trazado" que el usuario reportó al hacer **pushback**: el avión va
 * hacia atrás y luego taxía hacia adelante (≈180° de reversa), y el
 * spline uniforme dibujaba un bucle saliente que parecía una segunda
 * línea.
 *
 * La variante CENTRÍPETA (Catmull-Rom, α = 0.5) está matemáticamente
 * garantizada SIN lazos, cúspides ni auto-intersecciones — sigue
 * pasando exactamente por cada punto de entrada. Implementación por la
 * fórmula piramidal de Barry–Goldman.
 *
 * `segmentsPerStep` controla cuántos puntos intermedios se generan
 * entre cada par de puntos de entrada. Default 8.
 */
export function smoothCatmullRom(
  points: LngLat[],
  segmentsPerStep = 8,
): LngLat[] {
  // Dedupe de puntos consecutivos (casi) idénticos. El avión parado
  // (pushback en pausa, esperando en plataforma) genera puntos
  // repetidos que harían el espaciado de knots degenerado (t_i==t_{i+1}).
  const pts: LngLat[] = [];
  for (const p of points) {
    const last = pts[pts.length - 1];
    if (
      !last ||
      Math.abs(last[0] - p[0]) > 1e-9 ||
      Math.abs(last[1] - p[1]) > 1e-9
    ) {
      pts.push(p);
    }
  }
  if (pts.length < 3) return pts.slice();

  // Extremos REFLEJADOS (en vez de duplicados) para que la curva pase
  // por start/end SIN crear un intervalo de knot de longitud cero.
  const first: LngLat = [
    2 * pts[0][0] - pts[1][0],
    2 * pts[0][1] - pts[1][1],
  ];
  const n = pts.length;
  const last: LngLat = [
    2 * pts[n - 1][0] - pts[n - 2][0],
    2 * pts[n - 1][1] - pts[n - 2][1],
  ];
  const ext: LngLat[] = [first, ...pts, last];

  const ALPHA = 0.5; // centrípeta
  const knot = (ti: number, a: LngLat, b: LngLat): number => {
    const d = Math.hypot(b[0] - a[0], b[1] - a[1]);
    return ti + Math.pow(Math.max(d, 1e-9), ALPHA);
  };

  const result: LngLat[] = [];
  for (let i = 1; i < ext.length - 2; i++) {
    const p0 = ext[i - 1];
    const p1 = ext[i];
    const p2 = ext[i + 1];
    const p3 = ext[i + 2];
    const t0 = 0;
    const t1 = knot(t0, p0, p1);
    const t2 = knot(t1, p1, p2);
    const t3 = knot(t2, p2, p3);
    for (let s = 0; s < segmentsPerStep; s++) {
      const t = t1 + (s / segmentsPerStep) * (t2 - t1);
      result.push([
        centripetalComponent(p0[0], p1[0], p2[0], p3[0], t0, t1, t2, t3, t),
        centripetalComponent(p0[1], p1[1], p2[1], p3[1], t0, t1, t2, t3, t),
      ]);
    }
  }
  result.push(pts[n - 1]); // último punto exacto
  return result;
}

/**
 * Componente escalar de la spline Catmull-Rom centrípeta vía la fórmula
 * piramidal de Barry–Goldman, evaluada en el parámetro `t ∈ [t1, t2]`
 * con knots no uniformes `t0 < t1 < t2 < t3`.
 */
function centripetalComponent(
  p0: number,
  p1: number,
  p2: number,
  p3: number,
  t0: number,
  t1: number,
  t2: number,
  t3: number,
  t: number,
): number {
  const a1 = ((t1 - t) / (t1 - t0)) * p0 + ((t - t0) / (t1 - t0)) * p1;
  const a2 = ((t2 - t) / (t2 - t1)) * p1 + ((t - t1) / (t2 - t1)) * p2;
  const a3 = ((t3 - t) / (t3 - t2)) * p2 + ((t - t2) / (t3 - t2)) * p3;
  const b1 = ((t2 - t) / (t2 - t0)) * a1 + ((t - t0) / (t2 - t0)) * a2;
  const b2 = ((t3 - t) / (t3 - t1)) * a2 + ((t - t1) / (t3 - t1)) * a3;
  return ((t2 - t) / (t2 - t1)) * b1 + ((t - t1) / (t2 - t1)) * b2;
}
