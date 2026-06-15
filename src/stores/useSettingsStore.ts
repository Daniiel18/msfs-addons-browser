import { create } from "zustand";
import type { AppSettings } from "../lib/types";
import { api } from "../lib/tauri";
import { persistLocale, setActiveLocale } from "../lib/i18n";
import {
  loadStoredUnitPrefs,
  persistUnitPrefs,
  IMPERIAL_PREFS,
  METRIC_PREFS,
  type UnitPrefs,
} from "../lib/unitsStorage";

/** (v3.39.0 #3) Campos de unidad en `AppSettings` (clave del store). */
type UnitField =
  | "unitWeight"
  | "unitAltitude"
  | "unitSpeed"
  | "unitVs"
  | "unitDistance"
  | "unitPressure"
  | "tempUnit";

/** Extrae las `UnitPrefs` (forma corta) desde el objeto de settings. */
function prefsOf(s: {
  unitWeight: string;
  unitAltitude: string;
  unitSpeed: string;
  unitVs: string;
  unitDistance: string;
  unitPressure: string;
  tempUnit: string;
}): UnitPrefs {
  return {
    weight: s.unitWeight as UnitPrefs["weight"],
    altitude: s.unitAltitude as UnitPrefs["altitude"],
    speed: s.unitSpeed as UnitPrefs["speed"],
    vs: s.unitVs as UnitPrefs["vs"],
    distance: s.unitDistance as UnitPrefs["distance"],
    pressure: s.unitPressure as UnitPrefs["pressure"],
    temp: s.tempUnit as UnitPrefs["temp"],
  };
}

/** Convierte `UnitPrefs` → los campos de unidad de `AppSettings`. */
function unitFields(p: UnitPrefs) {
  return {
    unitWeight: p.weight,
    unitAltitude: p.altitude,
    unitSpeed: p.speed,
    unitVs: p.vs,
    unitDistance: p.distance,
    unitPressure: p.pressure,
    tempUnit: p.temp,
  };
}

/**
 * Settings de la app — preferencias persistidas + autostart con
 * Windows. La fuente de verdad es el backend (tabla `settings` +
 * registro de Windows para autostart). Esta store es sólo cache
 * para que la UI sea reactiva.
 *
 * Patrón de actualización:
 *   1. Componente llama a `update({ key: value })`.
 *   2. La store hace un setState optimista.
 *   3. Backend persiste; si falla, revertimos y dejamos `lastError`.
 */
interface SettingsState {
  loaded: boolean;
  settings: AppSettings;
  lastError: string | null;

  bootstrap: () => Promise<void>;
  setBoolean: (
    key:
      | "showSimconnectLines"
      | "checkUpdatesOnStart",
    value: boolean,
  ) => Promise<void>;
  setDefaultView: (view: string) => Promise<void>;
  setLanguage: (lang: "auto" | "es" | "en") => Promise<void>;
  setUnitPref: (field: UnitField, value: string) => Promise<void>;
  applyUnitPreset: (preset: "imperial" | "metric") => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<void>;
  setMinimizeToTray: (enabled: boolean) => Promise<void>;
  setOnboardingCompleted: (done: boolean) => Promise<void>;
  /** (v5.1.0) Fija la versión de MSFS activa. NO reinicia: el caller
   *  decide pedir confirmación + recargar para reaplicar todo. */
  setSimVersion: (v: "msfs2020" | "msfs2024") => Promise<void>;
  clearCaches: () => Promise<number>;
  resetSettings: () => Promise<number>;
}

const DEFAULTS: AppSettings = {
  showSimconnectLines: true,
  checkUpdatesOnStart: true,
  minimizeToTray: false,
  onboardingCompleted: false,
  defaultView: "dashboard",
  theme: "dark",
  language: "auto",
  unitWeight: "lb",
  unitAltitude: "ft",
  unitSpeed: "kt",
  unitVs: "fpm",
  unitDistance: "nm",
  unitPressure: "inHg",
  tempUnit: "C",
  autostartEnabled: false,
  simbriefPilotId: null,
  communityPath: null,
  logsPath: null,
  appDataPath: null,
  simVersion: "",
};

const KEY_MAP: Record<string, string> = {
  showSimconnectLines: "pref_show_simconnect_lines",
  checkUpdatesOnStart: "pref_check_updates_on_start",
  minimizeToTray: "pref_minimize_to_tray",
  onboardingCompleted: "pref_onboarding_completed",
  defaultView: "pref_default_view",
  language: "pref_language",
  unitWeight: "pref_unit_weight",
  unitAltitude: "pref_unit_altitude",
  unitSpeed: "pref_unit_speed",
  unitVs: "pref_unit_vs",
  unitDistance: "pref_unit_distance",
  unitPressure: "pref_unit_pressure",
  tempUnit: "pref_temp_unit",
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  loaded: false,
  // (v3.28.0 P7.11) Sembramos unidades desde localStorage para que el
  // cold-start pinte en el sistema correcto ANTES del bootstrap async
  // de la DB (mismo patrón que el idioma con preloadLocale).
  settings: {
    ...DEFAULTS,
    ...unitFields(loadStoredUnitPrefs()),
  },
  lastError: null,

  async bootstrap() {
    try {
      const s = await api.getAppSettings();
      set({ settings: s, loaded: true });
      // (v3.4.0) Source of truth para el idioma es la DB. Tras
      // bootstrappear espejamos el valor al localStorage y al módulo
      // i18n — así un cold start de la app posterior arranca con el
      // idioma correcto SIN esperar el bootstrap. Cubre también el
      // caso "cloud sync bajó preferencias de otra PC con otro
      // idioma" — la DB cambia, este espejo lo refleja.
      const lang = (s.language ?? "auto") as "auto" | "es" | "en";
      persistLocale(lang);
      setActiveLocale(lang);
      // (v3.28.0 P7.11) Espejo de unidades a localStorage — la DB es la
      // fuente de verdad; esto sincroniza el cold-start y el caso de
      // cloud sync que bajó prefs de otra PC.
      persistUnitPrefs(prefsOf(s));
    } catch (e) {
      console.warn("settings bootstrap failed:", e);
      // Aun en error marcamos `loaded:true` para que la UI no se
      // bloquee — usaremos los defaults.
      set({ loaded: true, lastError: String(e) });
    }
  },

  async setBoolean(field, value) {
    const prev = get().settings;
    set({ settings: { ...prev, [field]: value } });
    const dbKey = KEY_MAP[field];
    try {
      await api.setAppSetting(dbKey, value ? "1" : "0");
    } catch (e) {
      // Revertir y reportar.
      set({ settings: prev, lastError: String(e) });
    }
  },

  async setDefaultView(view) {
    const prev = get().settings;
    set({ settings: { ...prev, defaultView: view } });
    try {
      await api.setAppSetting(KEY_MAP.defaultView, view);
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
    }
  },

  async setLanguage(lang) {
    // (v3.1.3) Write-first, then update store. Antes optimistic
    // update + write — si el write tardaba, un componente leyendo
    // state lo veía como "es" pero el DB seguía con "auto".
    // Tras restart, bootstrap leía DB ("auto") y revertía. Bug
    // reportado: "vuelve al estado anterior".
    //
    // (v3.4.0) Tras persistir a DB espejamos al localStorage. Sin
    // esto el siguiente cold start lee `preloadLocale()` con el
    // valor viejo y el shell arrancaba en el idioma anterior por
    // un frame (mismo bug que reportó el usuario "no traduce nada
    // al cambiar"). También llamamos `setActiveLocale` para que los
    // `t()` que se ejecuten antes del modal de restart ya devuelvan
    // strings del nuevo idioma.
    try {
      await api.setAppSetting(KEY_MAP.language, lang);
      persistLocale(lang);
      setActiveLocale(lang);
      const prev = get().settings;
      set({ settings: { ...prev, language: lang } });
      console.info(`[settings] language persisted: ${lang}`);
    } catch (e) {
      console.warn("setLanguage failed:", e);
      set({ lastError: String(e) });
      throw e;
    }
  },

  async setUnitPref(field, value) {
    // (v3.39.0 #3) Optimista + persiste + espeja. Reactivo: los
    // componentes que usan `useUnits()` se re-renderizan al instante.
    const prev = get().settings;
    const next = { ...prev, [field]: value };
    set({ settings: next });
    persistUnitPrefs(prefsOf(next));
    try {
      await api.setAppSetting(KEY_MAP[field], value);
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
      persistUnitPrefs(prefsOf(prev));
    }
  },

  async applyUnitPreset(preset) {
    // (v3.39.0 #3) Atajo: rellena las 6 magnitudes (no la temperatura)
    // desde un preset Imperial/Métrico en una sola pasada.
    const prev = get().settings;
    const base = preset === "metric" ? METRIC_PREFS : IMPERIAL_PREFS;
    const next = {
      ...prev,
      unitWeight: base.weight,
      unitAltitude: base.altitude,
      unitSpeed: base.speed,
      unitVs: base.vs,
      unitDistance: base.distance,
      unitPressure: base.pressure,
    };
    set({ settings: next });
    persistUnitPrefs(prefsOf(next));
    try {
      await Promise.all([
        api.setAppSetting("pref_unit_weight", base.weight),
        api.setAppSetting("pref_unit_altitude", base.altitude),
        api.setAppSetting("pref_unit_speed", base.speed),
        api.setAppSetting("pref_unit_vs", base.vs),
        api.setAppSetting("pref_unit_distance", base.distance),
        api.setAppSetting("pref_unit_pressure", base.pressure),
      ]);
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
      persistUnitPrefs(prefsOf(prev));
    }
  },

  async setAutostart(enabled) {
    const prev = get().settings;
    set({ settings: { ...prev, autostartEnabled: enabled } });
    try {
      const actual = await api.setAutostart(enabled);
      // El plugin devuelve el estado real tras el toggle — puede
      // haber rechazado por permisos. Lo reflejamos.
      set({ settings: { ...get().settings, autostartEnabled: actual } });
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
    }
  },

  async setMinimizeToTray(enabled) {
    const prev = get().settings;
    set({ settings: { ...prev, minimizeToTray: enabled } });
    try {
      await api.setAppSetting(KEY_MAP.minimizeToTray, enabled ? "1" : "0");
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
    }
  },

  async setOnboardingCompleted(done) {
    const prev = get().settings;
    set({ settings: { ...prev, onboardingCompleted: done } });
    try {
      await api.setAppSetting(KEY_MAP.onboardingCompleted, done ? "1" : "0");
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
    }
  },

  async setSimVersion(v) {
    // Write-first (como el idioma) para que un reinicio inmediato lea el
    // valor correcto de la DB. El caller recarga la app para reaplicar.
    try {
      await api.setAppSetting("pref_sim_version", v);
      const prev = get().settings;
      set({ settings: { ...prev, simVersion: v } });
    } catch (e) {
      set({ lastError: String(e) });
      throw e;
    }
  },

  async clearCaches() {
    try {
      return await api.clearCaches();
    } catch (e) {
      set({ lastError: String(e) });
      throw e;
    }
  },

  async resetSettings() {
    try {
      const n = await api.resetSettings();
      // Recargamos del backend para reflejar los defaults — más
      // robusto que reconstruir aquí.
      await get().bootstrap();
      return n;
    } catch (e) {
      set({ lastError: String(e) });
      throw e;
    }
  },
}));
