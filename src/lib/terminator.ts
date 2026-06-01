/**
 * (v4.0.0 — P6) Terminator line dinámica para el RoutesMapView.
 *
 * Computa el polígono GeoJSON que cubre la zona en penumbra/noche de
 * la Tierra para un timestamp UTC dado. Pintado como `fill-layer`
 * sobre el globo, divide visualmente las zonas iluminadas de las
 * oscuras — mismo efecto que se ve en Google Earth o Windy.
 *
 * ## Matemática
 *
 * El "terminator" es el círculo máximo donde el ángulo del sol sobre
 * el horizonte es 0°. Para cada longitud entre -180 y +180, calculamos
 * la latitud del terminator usando la fórmula:
 *
 *   tan(lat_terminator) = -cos(longitudHourAngle) / tan(declinacion)
 *
 * donde:
 *   · declinacion = ángulo del sol respecto al ecuador (-23.5° a +23.5°
 *     según la época del año).
 *   · longitudHourAngle = (longitud - subsolar_lon)°
 *   · subsolar_lon = -GMST × 15° (rotación de la Tierra contra el sol)
 *
 * `suncalc.getPosition(date, lat, lon)` nos da `altitude` directamente
 * para cada punto — alternativa más limpia que reimplementar la
 * trigonometría astronómica. Buscamos por bisección la latitud donde
 * altitude = 0 para cada longitud.
 *
 * ## Construcción del polígono
 *
 * El polígono "noche" se construye como un anillo cerrado:
 *   1. Para 181 longitudes (cada 2°), calcular la lat del terminator.
 *   2. Cerrar el contorno con los polos correspondientes según el
 *      hemisferio iluminado (verano vs invierno).
 *
 * ## Performance
 *
 * 181 puntos × 1 bisección por punto ≈ ~1ms de CPU. Despreciable.
 * Recomputar cada 60s — la Tierra rota 0.25°/min, así que un update
 * por minuto da un movimiento del terminator imperceptible al ojo
 * pero matemáticamente preciso.
 *
 * Renderizado: 1 fill-layer + 1 GeoJSON source. Sin impacto en FPS.
 */

import SunCalc from "suncalc";

/**
 * Calcula la latitud (°) del terminator para una longitud dada, en
 * el instante `date`. Devuelve `null` si el sol nunca se pone o nunca
 * sale en esa longitud (caso polar al solsticio).
 *
 * Algoritmo: bisección sobre `getPosition().altitude` entre -90 y +90.
 * En ~16 iteraciones la precisión es < 0.001° — más que suficiente
 * para resolución de mapa.
 */
function terminatorLatAtLon(
  date: Date,
  lon: number,
  altitudeThresholdRad: number = 0,
): number | null {
  // En cada longitud, buscamos la latitud donde altitude(lat) =
  // `altitudeThresholdRad`. Para el terminator estándar el threshold
  // es 0 (horizonte). Para civil twilight usamos -6° = -0.1047 rad,
  // que define el límite donde aún hay luz crepuscular suave en el
  // cielo. La altitude solar varía monótonamente con lat para una
  // longitud fija, así que la bisección converge en ~18 iteraciones.
  let lo = -90;
  let hi = 90;
  const aLo = SunCalc.getPosition(date, lo, lon).altitude - altitudeThresholdRad;
  const aHi = SunCalc.getPosition(date, hi, lon).altitude - altitudeThresholdRad;
  if (aLo * aHi > 0) return null;
  for (let i = 0; i < 18; i++) {
    const mid = (lo + hi) / 2;
    const aMid =
      SunCalc.getPosition(date, mid, lon).altitude - altitudeThresholdRad;
    if (aMid * aLo < 0) {
      hi = mid;
    } else {
      lo = mid;
    }
  }
  return (lo + hi) / 2;
}

/**
 * Construye el polígono GeoJSON de la zona en sombra (noche) para
 * el timestamp dado. El polígono cubre la mitad oscura de la Tierra
 * incluyendo los polos cuando aplica (polar night).
 *
 * El frontend usa el resultado en un MapLibre `GeoJSONSource.setData`
 * cada 60s para mover el terminator de manera continua.
 *
 * @param date Hora UTC (típicamente `new Date()` al momento del render).
 * @param step Espaciado en longitud entre samples (°). Default 2°
 *             produce 181 puntos — suficiente para que la curva se vea
 *             suave a cualquier zoom razonable.
 */
export function buildTerminatorPolygon(
  date: Date = new Date(),
  step: number = 2,
  altitudeDeg: number = 0,
): GeoJSON.Feature<GeoJSON.Polygon> {
  const altitudeRad = (altitudeDeg * Math.PI) / 180;
  // Sample del terminator cada `step`° de longitud, de -180 a +180.
  const terminatorRing: [number, number][] = [];
  for (let lon = -180; lon <= 180; lon += step) {
    const lat = terminatorLatAtLon(date, lon, altitudeRad);
    // Si no hay terminator en esta longitud (sol siempre arriba),
    // anclamos al polo más cercano al hemisferio iluminado para
    // mantener el polígono cerrado y continuo. La declinación solar
    // decide qué polo.
    if (lat == null) {
      const decl = SunCalc.getPosition(date, 0, lon).altitude - altitudeRad;
      terminatorRing.push([lon, decl > 0 ? 90 : -90]);
    } else {
      terminatorRing.push([lon, lat]);
    }
  }

  // ¿Qué hemisferio está oscuro? El hemisferio opuesto al que tiene
  // el sol arriba ahora. Usamos `getPosition` en el polo norte:
  // altitude > threshold → sol en el norte → noche en el sur.
  const northSunny =
    SunCalc.getPosition(date, 89, 0).altitude - altitudeRad > 0;

  // Para cerrar el polígono "noche" añadimos los segmentos verticales
  // a los polos. Si el norte está iluminado, la noche cubre desde el
  // terminator hacia el polo SUR.
  const polarLat = northSunny ? -90 : 90;
  // Closing path: ir hasta el polo a lon=+180, traversar el polo
  // (todos los puntos del polo son el mismo), volver al inicio.
  terminatorRing.push([180, polarLat]);
  terminatorRing.push([-180, polarLat]);
  terminatorRing.push(terminatorRing[0]);

  return {
    type: "Feature",
    properties: {},
    geometry: {
      type: "Polygon",
      coordinates: [terminatorRing],
    },
  };
}

/**
 * Devuelve el `Point` del lugar de la Tierra donde el sol está
 * directamente arriba en `date`. Útil para debug y para ubicar un
 * indicador opcional del "punto subsolar" en el mapa.
 */
export function subsolarPoint(date: Date = new Date()): {
  lat: number;
  lon: number;
} {
  // Bruteforce: el sol está donde altitude es máximo. Buscamos por
  // grid grueso + refine. Suficiente porque solo lo usamos para
  // mostrar un dot de debug — no necesita precisión de arcominutos.
  let bestLat = 0;
  let bestLon = 0;
  let bestAlt = -90;
  for (let lat = -23; lat <= 23; lat += 1) {
    for (let lon = -180; lon < 180; lon += 5) {
      const a = SunCalc.getPosition(date, lat, lon).altitude;
      if (a > bestAlt) {
        bestAlt = a;
        bestLat = lat;
        bestLon = lon;
      }
    }
  }
  return { lat: bestLat, lon: bestLon };
}
