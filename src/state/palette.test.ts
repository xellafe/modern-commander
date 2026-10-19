import { expect, test } from 'vitest';
import { usePalette } from './palette';

test('usePalette opens in a mode and closes', () => {
  expect(usePalette.getState().mode).toBeNull();
  usePalette.getState().open('commands');
  expect(usePalette.getState().mode).toBe('commands');
  usePalette.getState().open('jump');
  expect(usePalette.getState().mode).toBe('jump');
  usePalette.getState().close();
  expect(usePalette.getState().mode).toBeNull();
});
