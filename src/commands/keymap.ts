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

const isOverride = (o: unknown): o is KeybindingOverride =>
  typeof o === 'object' && o !== null &&
  typeof (o as KeybindingOverride).key === 'string' &&
  typeof (o as KeybindingOverride).command === 'string';

export async function loadKeymap(): Promise<Map<string, string>> {
  const raw = await readConfig('keybindings');
  return buildKeymap(DEFAULT_KEYMAP, Array.isArray(raw) ? raw.filter(isOverride) : []);
}
