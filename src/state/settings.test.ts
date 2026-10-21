import { expect, test } from 'vitest';
import { defaultSettings, resolveTheme } from './settings';
import { DEFAULT_COLUMNS } from '../lib/columns';
import en from '../i18n/en.json';
import it from '../i18n/it.json';

test('defaults follow OS locale', () => {
  expect(defaultSettings('it-IT').language).toBe('it');
  expect(defaultSettings('de-DE').language).toBe('en');
  expect(defaultSettings('en-US')).toMatchObject({ theme: 'system', showHidden: false, showSystem: false, confirmDelete: true, externalEditor: 'notepad.exe', useExternalEditor: false, terminalShell: 'auto', jumpInNewTab: false, columns: DEFAULT_COLUMNS, extInName: false });
});

test('resolveTheme', () => {
  expect(resolveTheme('system', true)).toBe('dark');
  expect(resolveTheme('system', false)).toBe('light');
  expect(resolveTheme('light', true)).toBe('light');
});

test('en and it have identical key sets', () => {
  const keys = (o: object, p = ''): string[] => Object.entries(o).flatMap(([k, v]) => typeof v === 'object' ? keys(v, p + k + '.') : [p + k]);
  expect(keys(it).sort()).toEqual(keys(en).sort());
});
