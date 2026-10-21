import { beforeEach, expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn() }));
import { readConfig } from '../ipc/api';
import { DEFAULT_KEYMAP } from './tcKeymap';
import { buildKeymap, loadKeymap } from './keymap';
import { registerNavigationCommands } from './navigation';
import { registerPaletteCommands } from './palette';
import { getCommand } from './registry';
import { registerTerminalCommands } from './terminal';
const read = vi.mocked(readConfig);
beforeEach(() => {
  read.mockReset();
});

test('overrides replace and remove', () => {
  const m = buildKeymap({ F5: 'cm_Copy', F6: 'cm_RenMov' }, [{ key: 'Ctrl+K', command: 'cm_Copy' }, { key: 'F6', command: '-cm_RenMov' }]);
  expect(m.get('Ctrl+K')).toBe('cm_Copy');
  expect(m.get('F5')).toBe('cm_Copy');
  expect(m.has('F6')).toBe(false);
});

test('removal only hits the named binding', () => {
  const m = buildKeymap({ F6: 'cm_RenMov' }, [{ key: 'F6', command: '-cm_Copy' }]);
  expect(m.get('F6')).toBe('cm_RenMov');
});

test('every default command id is namespaced', () => {
  for (const id of Object.values(DEFAULT_KEYMAP)) expect(id).toMatch(/^(cm|mc)_/);
});

test('loadKeymap merges overrides and ignores malformed entries', async () => {
  read.mockResolvedValue([{ key: 'F5', command: 'cm_Exit' }, { key: 1, command: 'x' }, null, { key: 'F9' }, 'junk']);
  const m = await loadKeymap();
  expect(m.get('F5')).toBe('cm_Exit');
  expect(m.get('F7')).toBe('cm_MkDir');
  expect(m.size).toBe(Object.keys(DEFAULT_KEYMAP).length);
});

test('loadKeymap falls back to defaults on missing or non-array config', async () => {
  read.mockResolvedValue(null);
  expect((await loadKeymap()).get('F5')).toBe('cm_Copy');
  read.mockResolvedValue({ F5: 'cm_Exit' });
  expect((await loadKeymap()).get('F5')).toBe('cm_Copy');
});

// Tasti aggiunti nella fase 2 (verificati nel Task 2, Step 1) con il contesto in cui valgono.
const PHASE2_KEYS: Array<[chord: string, id: string, when: 'group' | 'fileList' | 'global']> = [
  ['Ctrl+Q', 'cm_SrcQuickview', 'group'],
  ['Ctrl+Shift+F1', 'cm_SrcThumbs', 'fileList'],
  ['Ctrl+Shift+P', 'mc_CommandPalette', 'global'],
  ['Ctrl+P', 'mc_QuickJump', 'group'],
  ['Ctrl+Oem3', 'mc_ToggleTerminal', 'global'],
  ['Ctrl+Shift+Oem3', 'mc_NewTerminal', 'global'],
];

test('phase 2 keys are in the default keymap', () => {
  for (const [chord, id] of PHASE2_KEYS) expect(DEFAULT_KEYMAP[chord], chord).toBe(id);
});

test('F2 renames like Explorer, besides Shift+F6 of TC', () => {
  expect(DEFAULT_KEYMAP.F2).toBe('cm_RenameOnly');
  expect(DEFAULT_KEYMAP['Shift+F6']).toBe('cm_RenameOnly');
});

test('no chord is bound twice (case and modifier order do not matter)', () => {
  const norm = (c: string) => {
    // l'ultimo '+' è il tasto stesso se il chord finisce per '+' ('Ctrl++')
    const key = c.endsWith('+') ? '+' : c.slice(c.lastIndexOf('+') + 1);
    const mods = c.slice(0, c.length - key.length).split('+').filter(Boolean);
    return [...mods.map((m) => m.toLowerCase()).sort(), key.toLowerCase()].join('+');
  };
  const chords = Object.keys(DEFAULT_KEYMAP).map(norm);
  expect(chords.filter((c, i) => chords.indexOf(c) !== i)).toEqual([]);
});

test('mc_ToggleBottomPanel has no default key (replaced by mc_ToggleTerminal)', () => {
  expect(Object.values(DEFAULT_KEYMAP)).not.toContain('mc_ToggleBottomPanel');
});

test('phase 2 commands are registered with the expected context and have a title in both languages', async () => {
  registerNavigationCommands();
  registerPaletteCommands();
  registerTerminalCommands();
  const en = (await import('../i18n/en.json')).default as { commands: Record<string, string> };
  const it = (await import('../i18n/it.json')).default as { commands: Record<string, string> };
  for (const [, id, when] of PHASE2_KEYS) {
    const cmd = getCommand(id);
    expect(cmd, id).toBeDefined();
    expect(cmd?.when, id).toBe(when);
    expect(cmd?.titleKey, id).toBe(`commands.${id}`);
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
});
