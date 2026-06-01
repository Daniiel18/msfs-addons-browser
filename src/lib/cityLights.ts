/**
 * (v4.0.0 — P6) Dataset estático de las ~150 metrópolis más grandes
 * del mundo, para renderizar como "city lights" estilo satélite
 * (efecto NASA Black Marble) sobre el globo del FlightBook.
 *
 * **Por qué dataset estático vs NASA GIBS tiles**:
 *
 *   - NASA GIBS sirve tiles del "VIIRS Black Marble" — preciosos
 *     pero pesados (~50 KB cada uno × varios al hacer pan/zoom).
 *   - Más crítico: MapLibre no permite clippear raster layers por un
 *     polígono dinámico (solo `clip` para symbol/3d-model en v5).
 *     No hay manera limpia de mostrar Black Marble SOLO en la zona
 *     nocturna sin tapar el basemap diurno.
 *   - Dataset estático = ~10 KB inline, sin red, dots glow controlados
 *     por CSS/Paint, contraste natural (apenas visibles en día por
 *     bajo contraste contra Carto Voyager, fuertes en noche por
 *     contraste contra el shadow oscuro).
 *
 * ## Selección
 *
 * ~150 metrópolis curadas: capitales nacionales + ciudades con
 * población metropolitana >2M. Incluye representación de todos los
 * continentes para que el globo se vea "lleno de vida" en cualquier
 * rotación.
 *
 * `pop` aproximada en millones — el frontend la usa para escalar el
 * radio del dot (más grande = ciudades con más luz visible desde
 * satélite).
 */

export interface CityLightFeature {
  name: string;
  /** Población metropolitana aproximada en millones. */
  pop: number;
  lon: number;
  lat: number;
}

const RAW: CityLightFeature[] = [
  // === Asia (densidad más alta del mundo) ===
  { name: "Tokyo", pop: 37, lon: 139.6917, lat: 35.6895 },
  { name: "Delhi", pop: 32, lon: 77.1025, lat: 28.7041 },
  { name: "Shanghai", pop: 28, lon: 121.4737, lat: 31.2304 },
  { name: "Dhaka", pop: 23, lon: 90.4125, lat: 23.8103 },
  { name: "Mumbai", pop: 21, lon: 72.8777, lat: 19.076 },
  { name: "Beijing", pop: 21, lon: 116.4074, lat: 39.9042 },
  { name: "Osaka", pop: 19, lon: 135.5023, lat: 34.6937 },
  { name: "Karachi", pop: 17, lon: 67.0099, lat: 24.8607 },
  { name: "Chongqing", pop: 17, lon: 106.5516, lat: 29.563 },
  { name: "Istanbul", pop: 16, lon: 28.9784, lat: 41.0082 },
  { name: "Kolkata", pop: 15, lon: 88.3639, lat: 22.5726 },
  { name: "Manila", pop: 15, lon: 120.9842, lat: 14.5995 },
  { name: "Tianjin", pop: 14, lon: 117.3616, lat: 39.3434 },
  { name: "Guangzhou", pop: 14, lon: 113.2644, lat: 23.1291 },
  { name: "Shenzhen", pop: 13, lon: 114.0579, lat: 22.5431 },
  { name: "Lahore", pop: 13, lon: 74.3587, lat: 31.5204 },
  { name: "Bangalore", pop: 13, lon: 77.5946, lat: 12.9716 },
  { name: "Jakarta", pop: 11, lon: 106.8456, lat: -6.2088 },
  { name: "Chennai", pop: 11, lon: 80.2707, lat: 13.0827 },
  { name: "Bangkok", pop: 10, lon: 100.5018, lat: 13.7563 },
  { name: "Seoul", pop: 10, lon: 126.978, lat: 37.5665 },
  { name: "Hyderabad", pop: 10, lon: 78.4867, lat: 17.385 },
  { name: "Tehran", pop: 9, lon: 51.389, lat: 35.6892 },
  { name: "Ho Chi Minh", pop: 9, lon: 106.6297, lat: 10.8231 },
  { name: "Ahmedabad", pop: 8, lon: 72.5714, lat: 23.0225 },
  { name: "Kuala Lumpur", pop: 8, lon: 101.6869, lat: 3.139 },
  { name: "Hong Kong", pop: 7, lon: 114.1694, lat: 22.3193 },
  { name: "Singapore", pop: 6, lon: 103.8198, lat: 1.3521 },
  { name: "Riyadh", pop: 7, lon: 46.6753, lat: 24.7136 },
  { name: "Baghdad", pop: 7, lon: 44.3661, lat: 33.3152 },
  { name: "Kabul", pop: 5, lon: 69.2075, lat: 34.5553 },
  { name: "Hanoi", pop: 5, lon: 105.8542, lat: 21.0285 },
  { name: "Yangon", pop: 5, lon: 96.1561, lat: 16.8409 },
  { name: "Tashkent", pop: 3, lon: 69.2401, lat: 41.2995 },
  { name: "Almaty", pop: 2, lon: 76.8512, lat: 43.222 },
  { name: "Pyongyang", pop: 3, lon: 125.7625, lat: 39.0392 },
  { name: "Ulaanbaatar", pop: 2, lon: 106.9057, lat: 47.8864 },
  { name: "Damascus", pop: 2, lon: 36.2765, lat: 33.5138 },
  { name: "Amman", pop: 4, lon: 35.9106, lat: 31.9454 },
  { name: "Beirut", pop: 2, lon: 35.5018, lat: 33.8938 },
  { name: "Jerusalem", pop: 1, lon: 35.2137, lat: 31.7683 },
  { name: "Tel Aviv", pop: 4, lon: 34.7818, lat: 32.0853 },
  { name: "Doha", pop: 3, lon: 51.531, lat: 25.2854 },
  { name: "Abu Dhabi", pop: 2, lon: 54.3773, lat: 24.4539 },
  { name: "Dubai", pop: 4, lon: 55.2708, lat: 25.2048 },
  { name: "Mecca", pop: 2, lon: 39.8579, lat: 21.4225 },
  { name: "Isfahan", pop: 2, lon: 51.668, lat: 32.6539 },
  { name: "Mashhad", pop: 3, lon: 59.6133, lat: 36.265 },
  { name: "Sapporo", pop: 3, lon: 141.3545, lat: 43.0618 },
  { name: "Fukuoka", pop: 6, lon: 130.4017, lat: 33.5904 },
  { name: "Nagoya", pop: 9, lon: 136.9066, lat: 35.1815 },

  // === Europa ===
  { name: "Moscow", pop: 13, lon: 37.6173, lat: 55.7558 },
  { name: "London", pop: 9, lon: -0.1276, lat: 51.5074 },
  { name: "Paris", pop: 11, lon: 2.3522, lat: 48.8566 },
  { name: "Madrid", pop: 7, lon: -3.7038, lat: 40.4168 },
  { name: "Saint Petersburg", pop: 5, lon: 30.3158, lat: 59.9343 },
  { name: "Barcelona", pop: 5, lon: 2.1734, lat: 41.3851 },
  { name: "Berlin", pop: 4, lon: 13.405, lat: 52.52 },
  { name: "Rome", pop: 4, lon: 12.4964, lat: 41.9028 },
  { name: "Athens", pop: 3, lon: 23.7275, lat: 37.9838 },
  { name: "Lisbon", pop: 3, lon: -9.1393, lat: 38.7223 },
  { name: "Vienna", pop: 2, lon: 16.3738, lat: 48.2082 },
  { name: "Hamburg", pop: 2, lon: 9.9937, lat: 53.5511 },
  { name: "Munich", pop: 2, lon: 11.582, lat: 48.1351 },
  { name: "Milan", pop: 3, lon: 9.19, lat: 45.4642 },
  { name: "Stockholm", pop: 2, lon: 18.0686, lat: 59.3293 },
  { name: "Helsinki", pop: 1, lon: 24.9384, lat: 60.1699 },
  { name: "Oslo", pop: 1, lon: 10.7522, lat: 59.9139 },
  { name: "Copenhagen", pop: 2, lon: 12.5683, lat: 55.6761 },
  { name: "Amsterdam", pop: 2, lon: 4.9041, lat: 52.3676 },
  { name: "Brussels", pop: 2, lon: 4.3517, lat: 50.8503 },
  { name: "Zurich", pop: 1, lon: 8.5417, lat: 47.3769 },
  { name: "Dublin", pop: 1, lon: -6.2603, lat: 53.3498 },
  { name: "Warsaw", pop: 2, lon: 21.0122, lat: 52.2297 },
  { name: "Prague", pop: 1, lon: 14.4378, lat: 50.0755 },
  { name: "Budapest", pop: 2, lon: 19.0402, lat: 47.4979 },
  { name: "Bucharest", pop: 2, lon: 26.1025, lat: 44.4268 },
  { name: "Kyiv", pop: 3, lon: 30.5234, lat: 50.4501 },
  { name: "Minsk", pop: 2, lon: 27.5615, lat: 53.9006 },
  { name: "Riga", pop: 1, lon: 24.1052, lat: 56.9496 },
  { name: "Sofia", pop: 1, lon: 23.3219, lat: 42.6977 },
  { name: "Belgrade", pop: 2, lon: 20.4489, lat: 44.7866 },
  { name: "Reykjavik", pop: 0.2, lon: -21.9426, lat: 64.1466 },
  { name: "Ankara", pop: 5, lon: 32.8597, lat: 39.9334 },

  // === América del Norte ===
  { name: "New York", pop: 18, lon: -74.006, lat: 40.7128 },
  { name: "Los Angeles", pop: 13, lon: -118.2437, lat: 34.0522 },
  { name: "Chicago", pop: 9, lon: -87.6298, lat: 41.8781 },
  { name: "Houston", pop: 7, lon: -95.3698, lat: 29.7604 },
  { name: "Dallas", pop: 7, lon: -96.797, lat: 32.7767 },
  { name: "Miami", pop: 6, lon: -80.1918, lat: 25.7617 },
  { name: "Atlanta", pop: 6, lon: -84.388, lat: 33.749 },
  { name: "Washington DC", pop: 6, lon: -77.0369, lat: 38.9072 },
  { name: "Philadelphia", pop: 6, lon: -75.1652, lat: 39.9526 },
  { name: "Phoenix", pop: 5, lon: -112.074, lat: 33.4484 },
  { name: "Boston", pop: 5, lon: -71.0589, lat: 42.3601 },
  { name: "Detroit", pop: 4, lon: -83.0458, lat: 42.3314 },
  { name: "Seattle", pop: 4, lon: -122.3321, lat: 47.6062 },
  { name: "San Francisco", pop: 4, lon: -122.4194, lat: 37.7749 },
  { name: "Denver", pop: 3, lon: -104.9903, lat: 39.7392 },
  { name: "Minneapolis", pop: 4, lon: -93.265, lat: 44.9778 },
  { name: "Honolulu", pop: 1, lon: -157.8583, lat: 21.3069 },
  { name: "Anchorage", pop: 0.3, lon: -149.9003, lat: 61.2181 },
  { name: "Mexico City", pop: 22, lon: -99.1332, lat: 19.4326 },
  { name: "Guadalajara", pop: 5, lon: -103.3496, lat: 20.6597 },
  { name: "Monterrey", pop: 5, lon: -100.3161, lat: 25.6866 },
  { name: "Toronto", pop: 7, lon: -79.3832, lat: 43.6532 },
  { name: "Montreal", pop: 4, lon: -73.5673, lat: 45.5017 },
  { name: "Vancouver", pop: 3, lon: -123.1207, lat: 49.2827 },
  { name: "Calgary", pop: 2, lon: -114.0719, lat: 51.0447 },

  // === América del Sur ===
  { name: "São Paulo", pop: 22, lon: -46.6333, lat: -23.5505 },
  { name: "Buenos Aires", pop: 15, lon: -58.3816, lat: -34.6037 },
  { name: "Rio de Janeiro", pop: 14, lon: -43.1729, lat: -22.9068 },
  { name: "Bogotá", pop: 11, lon: -74.0721, lat: 4.711 },
  { name: "Lima", pop: 11, lon: -77.0428, lat: -12.0464 },
  { name: "Santiago", pop: 7, lon: -70.6483, lat: -33.4569 },
  { name: "Belo Horizonte", pop: 6, lon: -43.9378, lat: -19.9167 },
  { name: "Caracas", pop: 5, lon: -66.9036, lat: 10.4806 },
  { name: "Brasília", pop: 4, lon: -47.9292, lat: -15.7975 },
  { name: "Quito", pop: 3, lon: -78.4678, lat: -0.1807 },
  { name: "Medellín", pop: 4, lon: -75.5812, lat: 6.2442 },
  { name: "Recife", pop: 4, lon: -34.8769, lat: -8.0578 },
  { name: "Porto Alegre", pop: 4, lon: -51.2177, lat: -30.0346 },
  { name: "Montevideo", pop: 2, lon: -56.1645, lat: -34.9011 },
  { name: "La Paz", pop: 2, lon: -68.1193, lat: -16.4897 },
  { name: "Asunción", pop: 3, lon: -57.6359, lat: -25.2637 },

  // === África ===
  { name: "Cairo", pop: 22, lon: 31.2357, lat: 30.0444 },
  { name: "Lagos", pop: 15, lon: 3.3792, lat: 6.5244 },
  { name: "Kinshasa", pop: 15, lon: 15.2663, lat: -4.4419 },
  { name: "Luanda", pop: 8, lon: 13.2317, lat: -8.839 },
  { name: "Dar es Salaam", pop: 7, lon: 39.2083, lat: -6.7924 },
  { name: "Khartoum", pop: 6, lon: 32.5599, lat: 15.5007 },
  { name: "Johannesburg", pop: 6, lon: 28.0473, lat: -26.2041 },
  { name: "Cape Town", pop: 5, lon: 18.4241, lat: -33.9249 },
  { name: "Casablanca", pop: 4, lon: -7.5898, lat: 33.5731 },
  { name: "Abidjan", pop: 5, lon: -4.0083, lat: 5.3599 },
  { name: "Nairobi", pop: 5, lon: 36.8219, lat: -1.2921 },
  { name: "Alexandria", pop: 5, lon: 29.9187, lat: 31.2001 },
  { name: "Addis Ababa", pop: 5, lon: 38.7578, lat: 9.0054 },
  { name: "Algiers", pop: 4, lon: 3.0588, lat: 36.7538 },
  { name: "Accra", pop: 3, lon: -0.187, lat: 5.6037 },
  { name: "Dakar", pop: 3, lon: -17.4677, lat: 14.7167 },
  { name: "Tunis", pop: 3, lon: 10.1815, lat: 36.8065 },
  { name: "Nouakchott", pop: 1, lon: -15.9785, lat: 18.0735 },

  // === Oceanía ===
  { name: "Sydney", pop: 5, lon: 151.2093, lat: -33.8688 },
  { name: "Melbourne", pop: 5, lon: 144.9631, lat: -37.8136 },
  { name: "Brisbane", pop: 3, lon: 153.0251, lat: -27.4698 },
  { name: "Perth", pop: 2, lon: 115.8605, lat: -31.9505 },
  { name: "Adelaide", pop: 1, lon: 138.6007, lat: -34.9285 },
  { name: "Auckland", pop: 2, lon: 174.7633, lat: -36.8485 },
  { name: "Wellington", pop: 0.4, lon: 174.7762, lat: -41.2865 },
];

/**
 * GeoJSON FeatureCollection con todas las ciudades. Lazy-generado al
 * primer import para evitar overhead si nunca se renderiza el globo.
 */
export const cityLightsGeoJSON: GeoJSON.FeatureCollection<GeoJSON.Point> = {
  type: "FeatureCollection",
  features: RAW.map((c) => ({
    type: "Feature",
    properties: { name: c.name, pop: c.pop },
    geometry: { type: "Point", coordinates: [c.lon, c.lat] },
  })),
};
