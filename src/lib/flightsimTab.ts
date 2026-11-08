import { useAppStore } from "../stores/useAppStore";
import { useFstoEmbedStore } from "../components/FlightsimEmbed";

/**
 * (26/10) Abre una página de flightsim.to en su pestaña de Explorar (no en la
 * ventana aparte), como las tarjetas de actualizaciones.
 */
export function openInFlightsimTab(url: string) {
  useFstoEmbedStore.getState().openUrl(url);
  useAppStore.getState().openFlightsimTab();
}

/** (03/11) Búsqueda de liveries en flightsim.to (por matrícula). */
export const liverySearchUrl = (term: string) =>
  `https://flightsim.to/search?q=${encodeURIComponent(term)}&cat=liveries`;

/** Búsqueda de perfiles GSX de un ICAO en flightsim.to. */
export const gsxSearchUrl = (icao: string) =>
  `https://flightsim.to/miscellaneous/gsx-pro?q=${encodeURIComponent(icao)}`;
