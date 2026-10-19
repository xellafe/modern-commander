import { expect, test, vi } from 'vitest';
import type { KeyLike } from '../commands/keys';
import { createExits, createLink, passToShell, removeSession, resolveShell, toggleTerminal } from './terminals';

const s = (id: number) => ({ id, title: `t${id}`, shell: 'pwsh' as const });

test('removeSession', () => {
  expect(removeSession([s(1), s(2), s(3)], 2, 2)).toEqual({ sessions: [s(1), s(3)], activeId: 3 });
  expect(removeSession([s(1), s(2)], 2, 2).activeId).toBe(1);
  expect(removeSession([s(1)], 1, 1)).toEqual({ sessions: [], activeId: null });
  expect(removeSession([s(1), s(2)], 1, 2).activeId).toBe(1);
  // un id sconosciuto non cambia nulla
  expect(removeSession([s(1), s(2)], 2, 9)).toEqual({ sessions: [s(1), s(2)], activeId: 2 });
});

test('toggleTerminal', () => {
  expect(toggleTerminal({ visible: true, tab: 'terminal' }, true)).toEqual({ visible: false, tab: 'terminal', create: false });
  expect(toggleTerminal({ visible: true, tab: 'operations' }, true)).toEqual({ visible: true, tab: 'terminal', create: false });
  expect(toggleTerminal({ visible: false, tab: 'operations' }, false)).toEqual({ visible: true, tab: 'terminal', create: true });
});

test('passToShell keeps global shortcuts for the app', () => {
  const km = new Map([['Ctrl+Oem3', 'mc_ToggleTerminal'], ['Ctrl+Shift+P', 'mc_CommandPalette'], ['Ctrl+P', 'mc_QuickJump'], ['Tab', 'mc_FocusNextGroup']]);
  const global = (id: string) => id === 'mc_ToggleTerminal' || id === 'mc_CommandPalette';
  const k = (keyCode: number, m: Partial<KeyLike> = {}) => ({ keyCode, ctrlKey: false, shiftKey: false, altKey: false, ...m });
  expect(passToShell(k(192, { ctrlKey: true }), km, global)).toBe(false);
  expect(passToShell(k(80, { ctrlKey: true, shiftKey: true }), km, global)).toBe(false);
  expect(passToShell(k(80, { ctrlKey: true }), km, global)).toBe(true);
  expect(passToShell(k(9), km, global)).toBe(true);
  expect(passToShell(k(67, { ctrlKey: true }), km, global)).toBe(true);
});

test('resolveShell: auto segue il backend (pwsh, powershell, cmd), una shell assente ripiega su auto', () => {
  expect(resolveShell('auto', ['pwsh', 'cmd', 'wsl'])).toBe('pwsh');
  expect(resolveShell('auto', ['powershell', 'cmd'])).toBe('powershell');
  expect(resolveShell('auto', ['wsl'])).toBe('cmd');
  expect(resolveShell('wsl', ['pwsh', 'wsl'])).toBe('wsl');
  expect(resolveShell('wsl', ['pwsh'])).toBe('pwsh');
});

test('createLink: le scritture prima dell\'id restano in coda e partono nell\'ordine alla risoluzione', () => {
  const write = vi.fn();
  const resize = vi.fn();
  const link = createLink({ write, resize }, { cols: 80, rows: 24 });
  link.input('\x1b[1;1R'); // risposta di xterm alla richiesta di posizione del cursore
  link.input('a');
  expect(write).not.toHaveBeenCalled();
  link.resolve(7);
  expect(write.mock.calls).toEqual([[7, '\x1b[1;1R'], [7, 'a']]);
  link.input('b');
  expect(write).toHaveBeenLastCalledWith(7, 'b');
  expect(resize).not.toHaveBeenCalled(); // le dimensioni non sono cambiate
});

test('createLink: un ridimensionamento prima dell\'id viene inviato una volta sola, dopo la coda', () => {
  const calls: string[] = [];
  const link = createLink({ write: (id, d) => calls.push(`w${id}:${d}`), resize: (id, c, r) => calls.push(`r${id}:${c}x${r}`) }, { cols: 80, rows: 24 });
  link.resize(100, 30);
  link.resize(120, 40);
  link.input('x');
  link.resolve(3);
  expect(calls).toEqual(['w3:x', 'r3:120x40']);
  link.resize(90, 20);
  expect(calls[calls.length - 1]).toBe('r3:90x20');
});

test('createLink: tornare alle dimensioni iniziali prima dell\'id non invia nulla', () => {
  const resize = vi.fn();
  const link = createLink({ write: vi.fn(), resize }, { cols: 80, rows: 24 });
  link.resize(100, 30);
  link.resize(80, 24);
  link.resolve(1);
  expect(resize).not.toHaveBeenCalled();
});

test('createExits: term-exit prima della risoluzione viene gestito alla registrazione', () => {
  const known = new Set<number>();
  const onExit = vi.fn();
  const exits = createExits((id) => known.has(id), onExit);
  exits.exit(5); // id ancora sconosciuto
  expect(onExit).not.toHaveBeenCalled();
  known.add(5);
  exits.register(5);
  expect(onExit).toHaveBeenCalledWith(5);
  expect(onExit).toHaveBeenCalledTimes(1);
});

test('createExits: term-exit dopo la registrazione è immediato e una sola volta', () => {
  const known = new Set([6]);
  const onExit = vi.fn();
  const exits = createExits((id) => known.has(id), onExit);
  exits.register(6); // nessuna uscita precoce: niente
  expect(onExit).not.toHaveBeenCalled();
  exits.exit(6);
  expect(onExit).toHaveBeenCalledWith(6);
});
