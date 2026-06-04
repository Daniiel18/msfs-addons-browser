import { create } from "zustand";
import type { AppSettings } from "../lib/types";
import { api } from "../lib/tauri";
import { persistLocale, setActiveLocale } from "../lib/i18n";
import {
  loadStoredUnitSystem,
  loadStoredTempUnit,
  persistUnits,
} from "../lib/unitsStorage";

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
  setUnitSystem: (system: "imperial" | "metric") => Promise<void>;
  setTempUnit: (unit: "C" | "F") => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<void>;
  setMinimizeToTray: (enabled: boolean) => Promise<void>;
  setOnboardingCompleted: (done: boolean) => Promise<void>;
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
  unitSystem: "imperial",
  tempUnit: "C",
  autostartEnabled: false,
  simbriefPilotId: null,
  communityPath: null,
  logsPath: null,
  appDataPath: null,
};

const KEY_MAP: Record<string, string> = {
  showSimconnectLines: "pref_show_simconnect_lines",
  checkUpdatesOnStart: "pref_check_updates_on_start",
  minimizeToTray: "pref_minimize_to_tray",
  onboardingCompleted: "pref_onboarding_completed",
  defaultView: "pref_default_view",
  language: "pref_language",
  unitSystem: "pref_unit_system",
  tempUnit: "pref_temp_unit",
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  loaded: false,
  // (v3.28.0 P7.11) Sembramos unidades desde localStorage para que el
  // cold-start pinte en el sistema correcto ANTES del bootstrap async
  // de la DB (mismo patrón que el idioma con preloadLocale).
  settings: {
    ...DEFAULTS,
    unitSystem: loadStoredUnitSystem(),
    tempUnit: loadStoredTempUnit(),
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
      persistUnits(s.unitSystem ?? "imperial", s.tempUnit ?? "C");
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

  async setUnitSystem(system) {
    // (v3.28.0 P7.11) Optimista + persiste + espeja. Reactivo: los
    // componentes que usan `useUnits()` se re-renderizan al instante.
    const prev = get().settings;
    set({ settings: { ...prev, unitSystem: system } });
    persistUnits(system, prev.tempUnit);
    try {
      await api.setAppSetting(KEY_MAP.unitSystem, system);
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
      persistUnits(prev.unitSystem, prev.tempUnit);
    }
  },

  async setTempUnit(unit) {
    const prev = get().settings;
    set({ settings: { ...prev, tempUnit: unit } });
    persistUnits(prev.unitSystem, unit);
    try {
      await api.setAppSetting(KEY_MAP.tempUnit, unit);
    } catch (e) {
      set({ settings: prev, lastError: String(e) });
      persistUnits(prev.unitSystem, prev.tempUnit);
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
