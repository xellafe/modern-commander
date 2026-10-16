import { readConfig } from '../ipc/api';
import { DEFAULT_KEYMAP } from './tcKeymap';

export interface KeybindingOverride {
  key: string;
  command: string; // '-id' = rimuovi il binding key -> id
}

export function buildKeymap(defaults: Record<string, string>, overrides: KeybindingOverride[]): Map<string, string> {
  const map = new Map(Object.entries(defaults));
  for (const { key, command } of overrides) {
    if (command.startsWith('-')) {
      if (map.get(key) === command.slice(1)) map.delete(key);
    } else {
      map.set(key, command);
    }
  }
  return map;
}

/** Il comando che ha già `chord`, se è diverso da `commandId`; altrimenti null. */
export const findConflicts = (map: Map<string, string>, chord: string, commandId: string): string | null => {
  const owner = map.get(chord);
  return owner !== undefined && owner !== commandId ? owner : null;
};

export const chordsOf = (map: Map<string, string>, commandId: string): string[] =>
  [...map].filter(([, id]) => id === commandId).map(([chord]) => chord);

/** Copia di `map` in cui i tasti di `commandId` sono esattamente `chords`: chi li aveva prima li perde. */
export function setChords(map: Map<string, string>, commandId: string, chords: string[]): Map<string, string> {
  const next = new Map([...map].filter(([, id]) => id !== commandId));
  for (const chord of chords) next.set(chord, commandId);
  return next;
}

const defaultChordsOf = (commandId: string): string[] =>
  Object.entries(DEFAULT_KEYMAP).filter(([, id]) => id === commandId).map(([chord]) => chord);

/** Riporta i tasti di `commandId` a quelli di fabbrica (che si riprendono anche se ora sono di altri). */
export const resetCommand = (map: Map<string, string>, commandId: string): Map<string, string> =>
  setChords(map, commandId, defaultChordsOf(commandId));

/** I tasti del comando non sono quelli di fabbrica (origine «utente»). */
export const isCustomized = (map: Map<string, string>, commandId: string): boolean => {
  const key = (chords: string[]) => [...chords].sort().join('|');
  return key(chordsOf(map, commandId)) !== key(defaultChordsOf(commandId));
};

/** Differenza minima tra la mappa di fabbrica e `map`: prima le rimozioni, poi le assegnazioni. */
export function diffOverrides(defaults: Record<string, string>, map: Map<string, string>): KeybindingOverride[] {
  const removed = Object.entries(defaults)
    .filter(([chord, id]) => map.get(chord) !== id)
    .map(([key, id]) => ({ key, command: `-${id}` }));
  const added = [...map]
    .filter(([chord, id]) => defaults[chord] !== id)
    .map(([key, command]) => ({ key, command }));
  return [...removed, ...added];
}

const isOverride =(o: unknown): o is KeybindingOverride =>
  typeof o === 'object' && o !== null &&
  typeof (o as KeybindingOverride).key === 'string' &&
  typeof (o as KeybindingOverride).command === 'string';

export async function loadKeymap(): Promise<Map<string, string>> {
  const raw = await readConfig('keybindings');
  return buildKeymap(DEFAULT_KEYMAP, Array.isArray(raw) ? raw.filter(isOverride) : []);
}
