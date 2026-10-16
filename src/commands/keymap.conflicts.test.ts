import { expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn() }));
import { buildKeymap, chordsOf, diffOverrides, findConflicts, isCustomized, resetCommand, setChords } from './keymap';
import { DEFAULT_KEYMAP } from './tcKeymap';

const defaults = () => buildKeymap(DEFAULT_KEYMAP, []);
const entries = (m: Map<string, string>) => [...m].sort(([a], [b]) => a.localeCompare(b));

test('findConflicts returns the other command that owns the chord', () => {
  expect(findConflicts(defaults(), 'F5', 'cm_Exit')).toBe('cm_Copy');
});

test('findConflicts: same command or a free chord is not a conflict', () => {
  expect(findConflicts(defaults(), 'F5', 'cm_Copy')).toBeNull();
  expect(findConflicts(defaults(), 'Ctrl+K', 'cm_Copy')).toBeNull();
});

test('chordsOf lists every chord of a command', () => {
  expect(chordsOf(defaults(), 'cm_Delete').sort()).toEqual(['Delete', 'F8']);
  expect(chordsOf(defaults(), 'mc_Keybindings')).toEqual([]);
});

test('isCustomized tells user bindings from factory ones', () => {
  const m = defaults();
  expect(isCustomized(m, 'cm_Copy')).toBe(false);
  expect(isCustomized(m, 'mc_Keybindings')).toBe(false);
  expect(isCustomized(setChords(m, 'cm_Copy', ['Ctrl+K']), 'cm_Copy')).toBe(true);
  expect(isCustomized(setChords(m, 'mc_Keybindings', ['Ctrl+K']), 'mc_Keybindings')).toBe(true);
  expect(isCustomized(resetCommand(setChords(m, 'cm_Copy', []), 'cm_Copy'), 'cm_Copy')).toBe(false);
});

test('no changes means no overrides', () => {
  expect(diffOverrides(DEFAULT_KEYMAP, defaults())).toEqual([]);
});

test('remapping a command yields the minimal diff, reset brings it back to empty', () => {
  const changed = setChords(defaults(), 'cm_Copy', ['Ctrl+K']);
  expect(chordsOf(changed, 'cm_Copy')).toEqual(['Ctrl+K']);
  const overrides = diffOverrides(DEFAULT_KEYMAP, changed);
  expect(overrides).toEqual([{ key: 'F5', command: '-cm_Copy' }, { key: 'Ctrl+K', command: 'cm_Copy' }]);
  // il file riletto dà la stessa mappa
  expect(entries(buildKeymap(DEFAULT_KEYMAP, overrides))).toEqual(entries(changed));

  const back = resetCommand(changed, 'cm_Copy');
  expect(diffOverrides(DEFAULT_KEYMAP, back)).toEqual([]);
});

test('removing every binding is a plain removal override', () => {
  const m = setChords(defaults(), 'cm_MkDir', []);
  expect(diffOverrides(DEFAULT_KEYMAP, m)).toEqual([{ key: 'F7', command: '-cm_MkDir' }]);
});

test('Replace: the chord moves to the new command, the old owner loses it', () => {
  const m = setChords(defaults(), 'cm_Exit', ['F5']);
  expect(m.get('F5')).toBe('cm_Exit');
  expect(chordsOf(m, 'cm_Copy')).toEqual([]);
  const overrides = diffOverrides(DEFAULT_KEYMAP, m);
  expect(overrides).toContainEqual({ key: 'F5', command: '-cm_Copy' });
  expect(overrides).toContainEqual({ key: 'F5', command: 'cm_Exit' });
  expect(entries(buildKeymap(DEFAULT_KEYMAP, overrides))).toEqual(entries(m));
  // il reset del vecchio proprietario si riprende il tasto
  expect(resetCommand(m, 'cm_Copy').get('F5')).toBe('cm_Copy');
});

test('reset leaves other commands alone', () => {
  const m = setChords(setChords(defaults(), 'cm_Copy', ['Ctrl+K']), 'cm_MkDir', ['Ctrl+M']);
  const r = resetCommand(m, 'cm_Copy');
  expect(r.get('Ctrl+M')).toBe('cm_MkDir');
  expect(r.has('Ctrl+K')).toBe(false);
  expect(diffOverrides(DEFAULT_KEYMAP, r)).toEqual([{ key: 'F7', command: '-cm_MkDir' }, { key: 'Ctrl+M', command: 'cm_MkDir' }]);
});
