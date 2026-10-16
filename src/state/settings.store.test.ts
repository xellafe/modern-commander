import { beforeEach, expect, test, vi } from 'vitest';

vi.mock('../ipc/api', () => ({ readConfig: vi.fn(), writeConfig: vi.fn() }));
import { writeConfig } from '../ipc/api';
import { useSettings } from './settings';

const write = vi.mocked(writeConfig);
beforeEach(() => write.mockReset());

test('updates are persisted in call order, last one wins', async () => {
  const delays = [30, 0];
  write.mockImplementation(() => new Promise((r) => setTimeout(r, delays.shift())));
  const a = useSettings.getState().update({ showHidden: true });
  const b = useSettings.getState().update({ showHidden: false });
  await Promise.all([a, b]);
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
