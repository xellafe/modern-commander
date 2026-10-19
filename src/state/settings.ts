import { create } from "zustand";
import { readConfig, writeConfig } from "../ipc/api";
import type { ShellKind } from "../ipc/types";

export interface Settings {
  theme: "dark" | "light" | "system";
  language: "en" | "it";
  showHidden: boolean;
  showSystem: boolean;
  confirmDelete: boolean;
  /** F4 apre `externalEditor` invece dell'editor interno. */
  useExternalEditor: boolean;
  externalEditor: string;
  /** Shell dei nuovi terminali; `auto` = la migliore disponibile (pwsh, powershell, cmd). */
  terminalShell: "auto" | ShellKind;
}

export const defaultSettings = (osLocale: string): Settings => ({
  theme: "system",
  language: osLocale.toLowerCase().startsWith("it") ? "it" : "en",
  showHidden: false,
  showSystem: false,
  confirmDelete: true,
  useExternalEditor: false,
  externalEditor: "notepad.exe",
  terminalShell: "auto",
});

export const resolveTheme = (theme: Settings["theme"], prefersDark: boolean): "dark" | "light" =>
  theme === "system" ? (prefersDark ? "dark" : "light") : theme;

interface SettingsState extends Settings {
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
}

const pickSettings = (s: Settings): Settings => ({
  theme: s.theme,
  language: s.language,
  showHidden: s.showHidden,
  showSystem: s.showSystem,
  confirmDelete: s.confirmDelete,
  useExternalEditor: s.useExternalEditor,
  externalEditor: s.externalEditor,
  terminalShell: s.terminalShell,
});

// Le scritture partono in ordine di chiamata: l'ultimo update() vince sempre.
let pending: Promise<unknown> = Promise.resolve();

export const useSettings = create<SettingsState>((set, get) => ({
  ...defaultSettings(navigator.language),
  load: async () => {
    const defaults = defaultSettings(navigator.language);
    const stored = (await readConfig("settings")) as Partial<Settings> | null;
    // Si fondono solo le chiavi note: le sconosciute sono ignorate, le nuove prendono il default.
    const merged = { ...defaults };
    for (const k of Object.keys(defaults) as (keyof Settings)[]) {
      if (stored && k in stored) (merged as Record<string, unknown>)[k] = stored[k];
    }
    set(merged);
  },
  update: async (patch) => {
    set(patch);
    const snapshot = pickSettings(get());
    const run = pending.then(() => writeConfig("settings", snapshot));
    pending = run.catch(() => {}); // un errore non blocca le scritture successive
    return run;
  },
}));
