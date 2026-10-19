import { expect, test, vi } from 'vitest';
import { initialLayout, openFileTab, useLayout } from '../state/layout';
import { executeCommand, isRunnable, registerCommand, shouldExecute, type Command } from './registry';

const cmd = (o: Partial<Command> = {}): Command => ({ id: 'cm_T', titleKey: 'x', when: 'fileList', run: () => {}, ...o });
const press = { repeat: false, focusInText: false, dirTab: true };

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

test('command context', () => {
  const c = (when: Command['when']) => cmd({ when });
  const o = { repeat: false, focusInText: false, dirTab: false };
  expect(shouldExecute(c('fileList'), o)).toBe(false);
  expect(shouldExecute(c('group'), o)).toBe(true);
  expect(shouldExecute(c('group'), { ...o, focusInText: true })).toBe(false);
  expect(shouldExecute(c('global'), { ...o, focusInText: true })).toBe(true);
  expect(shouldExecute(c('fileList'), { ...o, dirTab: true })).toBe(true);
});

test('executeCommand refuses a fileList command on a file tab (F-key bar, palette), not group or global ones', async () => {
  const saved = useLayout.getState(); // lo store è condiviso: la scheda di file non deve restarvi attiva
  const s = initialLayout(['C:\\a']);
  useLayout.setState(openFileTab(s, s.groups[0].id, { path: 'C:\\a\\f.txt', mode: 'viewer' }));
  try {
    const [list, group, global] = [vi.fn(), vi.fn(), vi.fn()];
    registerCommand(cmd({ id: 'cm_CtxList', when: 'fileList', run: list }));
    registerCommand(cmd({ id: 'cm_CtxGroup', when: 'group', run: group }));
    registerCommand(cmd({ id: 'cm_CtxGlobal', when: 'global', run: global }));
    expect(isRunnable(cmd({ when: 'fileList' }))).toBe(false);
    expect(isRunnable(cmd({ when: 'group' }))).toBe(true);
    for (const id of ['cm_CtxList', 'cm_CtxGroup', 'cm_CtxGlobal']) await executeCommand(id);
    expect([list, group, global].map((f) => f.mock.calls.length)).toEqual([0, 1, 1]);
  } finally {
    useLayout.setState(saved);
  }
});
