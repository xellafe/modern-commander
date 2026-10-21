import { expect, test } from 'vitest';
import { activeTab, initialLayout, openFileTab, useLayout } from '../state/layout';
import { usePalette } from '../state/palette';
import { DEFAULT_KEYMAP } from './tcKeymap';
import { useUi } from '../state/ui';
import { jumpItems, jumpTab, jumpTarget, paletteCommands, quickJump, togglePalette, wantsNewTab } from './palette';

test('wantsNewTab: Ctrl inverts the setting', () => {
  expect(wantsNewTab(false, false)).toBe(false);
  expect(wantsNewTab(true, false)).toBe(true);
  expect(wantsNewTab(false, true)).toBe(true);
  expect(wantsNewTab(true, true)).toBe(false);
});

test('jumpTab: a new folder tab right after the active one, in the same group', () => {
  const saved = useLayout.getState();
  const s = initialLayout(['C:\\a']);
  useLayout.setState(s);
  try {
    const g = s.groups[0];
    const first = g.activeTabId;
    const id = jumpTab('C:\\b', true);
    const l = useLayout.getState();
    expect(l.groups[0].tabIds).toEqual([first, id]);
    expect(l.groups).toHaveLength(s.groups.length);
    expect(activeTab(l).id).toBe(id);
    expect(l.tabs[id].path).toBe('C:\\b');
    expect(jumpTab('C:\\c', false)).toBe(id); // senza nuova scheda: quella attiva
    expect(useLayout.getState().groups[0].tabIds).toHaveLength(2);
  } finally {
    useLayout.setState(saved);
  }
});
import type { Command } from './registry';

const cmd = (id: string, when: Command['when']): Command => ({ id, titleKey: id, when, run: () => {} });

test('paletteCommands hides itself and shows chords', () => {
  const cmds = [cmd('cm_Copy', 'fileList'), cmd('mc_CommandPalette', 'global')];
  const items = paletteCommands(cmds, new Map([['F5', 'cm_Copy']]), (k) => k);
  expect(items.map((i) => i.id)).toEqual(['cm_Copy']);
  expect(items[0].keys).toEqual([['F5']]);
});

test('paletteCommands uses the translated title and splits chords into keys', () => {
  const items = paletteCommands([cmd('mc_SplitGroup', 'group')], new Map([['Ctrl+Shift+Oem5', 'mc_SplitGroup']]), (k) => `T:${k}`);
  expect(items[0].label).toBe('T:mc_SplitGroup');
  expect(items[0].keys).toEqual([['Ctrl', 'Shift', '\\']]);
});

test('paletteCommands drops fileList commands when the active tab is a file tab', () => {
  const saved = useLayout.getState();
  const s = initialLayout(['C:\\a']);
  useLayout.setState(openFileTab(s, s.groups[0].id, { path: 'C:\\a\\f.txt', mode: 'viewer' }));
  try {
    const cmds = [cmd('cm_Copy', 'fileList'), cmd('mc_SplitGroup', 'group'), cmd('cm_Config', 'global')];
    expect(paletteCommands(cmds, new Map(), (k) => k).map((i) => i.id)).toEqual(['mc_SplitGroup', 'cm_Config']);
  } finally {
    useLayout.setState(saved);
  }
});

test('the palette shortcut is Ctrl+Shift+P and does not take a TC key', () => {
  expect(DEFAULT_KEYMAP['Ctrl+Shift+P']).toBe('mc_CommandPalette');
});

test('togglePalette opens the commands palette and closes it when already open', () => {
  togglePalette();
  expect(usePalette.getState().mode).toBe('commands');
  togglePalette();
  expect(usePalette.getState().mode).toBeNull();
});

test('jumpItems keeps recent, favorites, files order', () => {
  const items = jumpItems(['C:\\r'], [{ name: 'Fav', path: 'C:\\f' }], [{ rel: 'x\\y.txt', isDir: false }]);
  expect(items.map((i) => i.kind)).toEqual(['recent', 'favorite', 'file']);
  expect(items.map((i) => i.label)).toEqual(['C:\\r', 'Fav', 'x\\y.txt']);
});

test('jump shortcut is Ctrl+P', () => {
  expect(DEFAULT_KEYMAP['Ctrl+P']).toBe('mc_QuickJump');
});

test('quickJump opens the jump palette, but not over a dialog', () => {
  useUi.setState({ dialog: { kind: 'unsaved', tabId: 't' } });
  quickJump();
  expect(usePalette.getState().mode).toBeNull();
  useUi.setState({ dialog: null });
  quickJump();
  expect(usePalette.getState().mode).toBe('jump');
  usePalette.getState().close();
});

test('jumpTarget: folders open as they are, files open their folder with the file focused', () => {
  const root = 'C:\\r';
  expect(jumpTarget({ kind: 'recent', path: 'C:\\a', label: 'C:\\a' }, root)).toEqual({ dir: 'C:\\a' });
  expect(jumpTarget({ kind: 'favorite', path: 'C:\\f', label: 'F' }, root)).toEqual({ dir: 'C:\\f' });
  expect(jumpTarget({ kind: 'file', rel: 'sub', isDir: true, label: 'sub' }, root)).toEqual({ dir: 'C:\\r\\sub' });
  expect(jumpTarget({ kind: 'file', rel: 'sub\\a.txt', isDir: false, label: 'sub\\a.txt' }, root))
    .toEqual({ dir: 'C:\\r\\sub', focus: 'a.txt' });
  expect(jumpTarget({ kind: 'file', rel: 'a.txt', isDir: false, label: 'a.txt' }, 'C:\\'))
    .toEqual({ dir: 'C:\\', focus: 'a.txt' });
});
