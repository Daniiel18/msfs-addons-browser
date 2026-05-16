import { create } from "zustand";
import type { AppSettings } from "../lib/types";
import { api } from "../lib/tauri";

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
      | "showSimbriefLines"
      | "showSimconnectLines"
      | "checkUpdatesOnStart",
    value: boolean,
  ) => Promise<void>;
  setDefaultView: (view: string) => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<void>;
  setMinimizeToTray: (enabled: boolean) => Promise<void>;
  setOnboardingCompleted: (done: boolean) => Promise<void>;
  clearCaches: () => Promise<number>;
  resetSettings: () => Promise<number>;
}

const DEFAULTS: AppSettings = {
  showSimbriefLines: true,
  showSimconnectLines: true,
  checkUpdatesOnStart: true,
  minimizeToTray: false,
  onboardingCompleted: false,
  defaultView: "dashboard",
  theme: "dark",
  autostartEnabled: false,
  simbriefPilotId: null,
  communityPath: null,
  logsPath: null,
  appDataPath: null,
};

const KEY_MAP: Record<string, string> = {
  showSimbriefLines: "pref_show_simbrief_lines",
  showSimconnectLines: "pref_show_simconnect_lines",
  checkUpdatesOnStart: "pref_check_updates_on_start",
  minimizeToTray: "pref_minimize_to_tray",
  onboardingCompleted: "pref_onboarding_completed",
  defaultView: "pref_default_view",
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  loaded: false,
  settings: DEFAULTS,
  lastError: null,

  async bootstrap() {
    try {
      const s = await api.getAppSettings();
      set({ settings: s, loaded: true });
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
