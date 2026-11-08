import maplibregl, { type MapLayerMouseEvent } from "maplibre-gl";
import { api } from "./tauri";
import { t } from "./i18n";
import type { AirportMetar } from "./types";

/** Lo que tarda en salir el globo: cruzar el mapa con el ratón no pide nada. */
const HOVER_DELAY_MS = 450;
const PIN_LAYERS = ["package-point", "sim-airport-point"];

const CATEGORY_TONE: Record<string, string> = {
  VFR: "bg-emerald-600",
  MVFR: "bg-sky-600",
  IFR: "bg-rose-600",
  LIFR: "bg-fuchsia-600",
};

function line(text: string, className: string): HTMLParagraphElement {
  const p = document.createElement("p");
  p.className = className;
  p.textContent = text;
  return p;
}

function age(observedAt: number): string {
  const minutes = Math.max(0, Math.round(Date.now() / 60_000 - observedAt / 60));
  return minutes < 120
    ? t("map.weather.age_min", { n: minutes })
    : t("map.weather.age_h", { n: Math.round(minutes / 60) });
}

function wind(m: AirportMetar): string | null {
  if (m.windKt === null) return null;
  const from = m.windDir === "VRB" ? t("map.weather.variable") : `${m.windDir ?? ""}°`;
  return `${from} ${m.windKt} kt${m.gustKt ? ` G${m.gustKt}` : ""}`;
}

function weatherBlock(m: AirportMetar): HTMLElement {
  const box = document.createElement("div");
  const facts = document.createElement("p");
  facts.className = "mt-1 flex flex-wrap items-center gap-1.5";
  if (m.category) {
    const tag = document.createElement("span");
    tag.className = `rounded px-1 text-[10px] font-bold text-white ${CATEGORY_TONE[m.category] ?? "bg-slate-600"}`;
    tag.textContent = m.category;
    facts.append(tag);
  }
  const text = [m.tempC !== null ? `${m.tempC} °C` : null, wind(m), m.observedAt ? age(m.observedAt) : null]
    .filter(Boolean)
    .join(" · ");
  facts.append(document.createTextNode(text));
  box.append(facts, line(m.raw, "mt-1 font-mono text-[10px] leading-snug opacity-80"));
  return box;
}

function card(title: string, body: HTMLElement | string): HTMLElement {
  const root = document.createElement("div");
  root.append(line(title, "font-semibold"));
  root.append(typeof body === "string" ? line(body, "mt-1 opacity-80") : body);
  return root;
}

interface PinProps {
  icao?: string;
  name?: string;
  airportName?: string;
  title?: string;
  sim?: boolean;
}

export function pinTitle(p: PinProps): string {
  const name = p.airportName ?? p.name ?? p.title ?? "";
  return [p.icao, name, p.sim ? t("map.sim_airport") : null].filter(Boolean).join(" · ");
}

/** (06/11) Clima real del pin señalado. Solo se pide ese, tras una pausa. */
export function bindWeatherPopup(map: maplibregl.Map) {
  const popup = new maplibregl.Popup({
    closeButton: false,
    closeOnClick: false,
    offset: 10,
    maxWidth: "300px",
    className: "sf-map-popup",
  });
  let current: string | null = null;
  let timer: number | undefined;

  const hide = () => {
    window.clearTimeout(timer);
    popup.remove();
  };
  const over = (e: MapLayerMouseEvent) => {
    const feature = e.features?.[0];
    const props = feature?.properties as PinProps | undefined;
    const icao = props?.icao;
    if (!feature || !icao || icao === current) return;
    hide();
    current = icao;
    const title = pinTitle(props);
    const where = (feature.geometry as GeoJSON.Point).coordinates as [number, number];
    timer = window.setTimeout(() => {
      popup.setLngLat(where).setDOMContent(card(title, t("map.weather.loading"))).addTo(map);
      api
        .airportMetar(icao)
        .then((metar) => {
          if (current !== icao) return;
          popup.setDOMContent(card(title, metar ? weatherBlock(metar) : t("map.weather.none")));
        })
        .catch(() => {
          if (current === icao) popup.setDOMContent(card(title, t("map.weather.offline")));
        });
    }, HOVER_DELAY_MS);
  };
  for (const layer of PIN_LAYERS) {
    map.on("mousemove", layer, over);
    map.on("mouseleave", layer, () => {
      current = null;
      hide();
    });
  }
  // Un clic abre la tarjeta o el globo del pin: este se aparta hasta salir del pin.
  map.on("click", hide);
}

export type PlanEnd = "orig" | "dest";

/** (06/11) Globo al pulsar un pin del simulador: se puede volar desde o hasta él. */
export function simPinContent(title: string, onPlan: (end: PlanEnd) => void): HTMLElement {
  const root = card(title, t("map.plan.hint"));
  const row = document.createElement("div");
  row.className = "mt-1.5 flex gap-1.5";
  for (const end of ["orig", "dest"] as const) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "rounded-md border border-sf-border px-2 py-0.5 text-[11px] font-medium hover:bg-sf-surface";
    button.textContent = t(`map.plan.${end}`);
    button.addEventListener("click", () => onPlan(end));
    row.append(button);
  }
  root.append(row);
  return root;
}
