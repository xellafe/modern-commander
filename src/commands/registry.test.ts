import { expect, test, vi } from 'vitest';
import { executeCommand, registerCommand, shouldExecute, type Command } from './registry';

const cmd = (o: Partial<Command> = {}): Command => ({ id: 'cm_T', titleKey: 'x', when: 'fileList', run: () => {}, ...o });
const press = { repeat: false, focusInText: false };

test('executeCommand: registered runs, unregistered is a silent no-op', async () => {
  const run = vi.fn();
  registerCommand(cmd({ id: 'cm_Test', when: 'global', run }));
  await executeCommand('cm_Test');
  expect(run).toHaveBeenCalledOnce();
  await expect(executeCommand('cm_Nope')).resolves.toBeUndefined();
});

test('shouldExecute: unregistered command never runs', () => {
  expect(shouldExecute(undefined, press)).toBe(false);
});

test('shouldExecute: auto-repeat runs only with opt-in', () => {
  expect(shouldExecute(cmd(), press)).toBe(true);
  expect(shouldExecute(cmd(), { ...press, repeat: true })).toBe(false);
  expect(shouldExecute(cmd({ repeat: true }), { ...press, repeat: true })).toBe(true);
});

test('shouldExecute: focus in text field allows only global commands', () => {
  expect(shouldExecute(cmd({ when: 'fileList' }), { ...press, focusInText: true })).toBe(false);
  expect(shouldExecute(cmd({ when: 'global' }), { ...press, focusInText: true })).toBe(true);
});
