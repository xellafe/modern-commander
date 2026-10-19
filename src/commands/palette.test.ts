import { expect, test } from 'vitest';
import { initialLayout, openFileTab, useLayout } from '../state/layout';
import { usePalette } from '../state/palette';
import { DEFAULT_KEYMAP } from './tcKeymap';
import { paletteCommands, togglePalette } from './palette';
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
