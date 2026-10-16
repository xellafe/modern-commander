import { beforeEach, expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn(), writeConfig: vi.fn() }));
import { readConfig, writeConfig } from '../ipc/api';
import { useKeybindings } from './keybindings';

const read = vi.mocked(readConfig);
const write = vi.mocked(writeConfig);
beforeEach(async () => {
  read.mockReset().mockResolvedValue(null);
  write.mockReset().mockResolvedValue(undefined);
  await useKeybindings.getState().load();
});

test('load starts from the factory map plus the stored overrides', async () => {
  read.mockResolvedValue([{ key: 'F5', command: '-cm_Copy' }, { key: 'Ctrl+K', command: 'cm_Copy' }]);
  await useKeybindings.getState().load();
  const { keymap } = useKeybindings.getState();
  expect(keymap.get('Ctrl+K')).toBe('cm_Copy');
  expect(keymap.has('F5')).toBe(false);
});

test('remapping updates the live keymap immediately and writes the minimal overrides', async () => {
  const done = useKeybindings.getState().setChords('cm_Copy', ['Ctrl+K']);
  // prima ancora che la scrittura finisca la mappa viva è già aggiornata
  expect(useKeybindings.getState().keymap.get('Ctrl+K')).toBe('cm_Copy');
  await done;
  expect(write).toHaveBeenCalledWith('keybindings', [{ key: 'F5', command: '-cm_Copy' }, { key: 'Ctrl+K', command: 'cm_Copy' }]);
});

test('reset writes an empty override list again', async () => {
  await useKeybindings.getState().setChords('cm_Copy', ['Ctrl+K']);
  await useKeybindings.getState().reset('cm_Copy');
  expect(useKeybindings.getState().keymap.get('F5')).toBe('cm_Copy');
  expect(write).toHaveBeenLastCalledWith('keybindings', []);
});

test('writes keep the order of the calls, the last one wins', async () => {
  const events: string[] = [];
  const delays = [30, 0];
  write.mockImplementation((_, v) => {
    const id = (v as unknown[]).length ? 'A' : 'B';
    events.push(`start ${id}`);
    return new Promise((r) => setTimeout(() => { events.push(`end ${id}`); r(undefined); }, delays.shift()));
  });
  const a = useKeybindings.getState().setChords('cm_Copy', ['Ctrl+K']);
  const b = useKeybindings.getState().reset('cm_Copy');
  await Promise.all([a, b]);
  expect(events).toEqual(['start A', 'end A', 'start B', 'end B']);
});

test('a failed write does not block the next one', async () => {
  write.mockRejectedValueOnce(new Error('boom'));
  await expect(useKeybindings.getState().setChords('cm_Copy', [])).rejects.toThrow('boom');
  await expect(useKeybindings.getState().reset('cm_Copy')).resolves.toBeUndefined();
});
