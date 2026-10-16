import { beforeEach, expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn(), writeConfig: vi.fn() }));
import { writeConfig } from '../ipc/api';
import { useSettings } from './settings';

const write = vi.mocked(writeConfig);
beforeEach(() => {
  write.mockReset(); // niente return: vitest lo tratterebbe come funzione di cleanup
});

test('updates are persisted in call order, last one wins', async () => {
  const delays = [30, 0];
  const events: string[] = [];
  write.mockImplementation((_, v) => {
    const id = (v as { showHidden: boolean }).showHidden ? 'A' : 'B';
    events.push(`start ${id}`);
    return new Promise((r) => setTimeout(() => { events.push(`end ${id}`); r(undefined); }, delays.shift()));
  });
  const a = useSettings.getState().update({ showHidden: true });
  const b = useSettings.getState().update({ showHidden: false });
  await Promise.all([a, b]);
  // la seconda scrittura parte solo dopo la fine della prima
  expect(events).toEqual(['start A', 'end A', 'start B', 'end B']);
  expect(write).toHaveBeenCalledTimes(2);
  expect(write.mock.calls[0][1]).toMatchObject({ showHidden: true });
  expect(write.mock.calls[1][1]).toMatchObject({ showHidden: false });
});

test('a rejected write does not block the next one', async () => {
  write.mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce(undefined);
  await expect(useSettings.getState().update({ confirmDelete: false })).rejects.toThrow('boom');
  await expect(useSettings.getState().update({ confirmDelete: true })).resolves.toBeUndefined();
  expect(write).toHaveBeenCalledTimes(2);
});
