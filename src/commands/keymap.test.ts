import { beforeEach, expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn() }));
import { readConfig } from '../ipc/api';
import { DEFAULT_KEYMAP } from './tcKeymap';
import { buildKeymap, loadKeymap } from './keymap';
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
