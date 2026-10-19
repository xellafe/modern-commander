import { expect, test } from 'vitest';
import { chordFromEvent, chordParts, isBrowserAccelerator, quickSearchChar } from './keys';

const ev = (keyCode: number, m: { c?: boolean; s?: boolean; a?: boolean } = {}) => ({
  keyCode,
  ctrlKey: !!m.c,
  shiftKey: !!m.s,
  altKey: !!m.a,
});

test('chord normalization uses virtual keys', () => {
  expect(chordFromEvent(ev(192, { c: true }))).toBe('Ctrl+Oem3');
  expect(chordFromEvent(ev(116, { s: true }))).toBe('Shift+F5');
  expect(chordFromEvent(ev(107))).toBe('NumAdd');
  expect(chordFromEvent(ev(13, { s: true, a: true }))).toBe('Shift+Alt+Enter');
  expect(chordFromEvent(ev(65, { c: true, s: true, a: true }))).toBe('Ctrl+Shift+Alt+A');
});

test('named keys and the VK<n> fallback', () => {
  expect(chordFromEvent(ev(188, { c: true }))).toBe('Ctrl+Comma');
  expect(chordFromEvent(ev(220, { c: true }))).toBe('Ctrl+Oem5');
  expect(chordFromEvent(ev(109))).toBe('NumSubtract');
  expect(chordFromEvent(ev(106))).toBe('NumMultiply');
  expect(chordFromEvent(ev(48))).toBe('0');
  expect(chordFromEvent(ev(135))).toBe('F24');
  expect(chordFromEvent(ev(34))).toBe('PageDown');
  expect(chordFromEvent(ev(93))).toBe('Apps');
  expect(chordFromEvent(ev(186, { c: true }))).toBe('Ctrl+VK186');
});

const altGr = (keyCode: number) => ({ ...ev(keyCode, { c: true, a: true }), getModifierState: (k: string) => k === 'AltGraph' });

test('AltGr never produces a Ctrl+Alt chord', () => {
  expect(chordFromEvent(altGr(69))).toBe('AltGr+E');
  expect(chordFromEvent({ ...ev(69, { c: true, a: true }), getModifierState: () => false })).toBe('Ctrl+Alt+E');
});

const typed = (key: string, keyCode: number, m: { c?: boolean; a?: boolean; g?: boolean } = {}) => ({
  key, keyCode, ctrlKey: !!m.c, altKey: !!m.a, shiftKey: false, metaKey: false,
  getModifierState: (k: string) => k === 'AltGraph' && !!m.g,
});

test('quickSearchChar: letters, Ctrl+Alt+letter, never AltGr', () => {
  expect(quickSearchChar(typed('a', 65))).toBe('a');
  expect(quickSearchChar(typed('.', 190))).toBe('.');
  expect(quickSearchChar(typed('a', 65, { c: true, a: true }))).toBe('a');
  expect(quickSearchChar(typed('€', 69, { c: true, a: true, g: true }))).toBeNull();
  expect(quickSearchChar(typed('@', 192, { c: true, a: true, g: true }))).toBeNull();
  expect(quickSearchChar(typed('a', 65, { c: true }))).toBeNull();
  expect(quickSearchChar(typed(' ', 32))).toBeNull();
  expect(quickSearchChar(typed('Enter', 13))).toBeNull();
});

test('chordParts: readable key names for display', () => {
  expect(chordParts('F5')).toEqual(['F5']);
  expect(chordParts('Ctrl+Oem3')).toEqual(['Ctrl', '`']);
  expect(chordParts('Ctrl+Shift+Oem5')).toEqual(['Ctrl', 'Shift', '\\']);
  expect(chordParts('Ctrl+Comma')).toEqual(['Ctrl', ',']);
  expect(chordParts('Ctrl+NumAdd')).toEqual(['Ctrl', 'Num +']);
  expect(chordParts('NumSubtract')).toEqual(['Num -']);
  expect(chordParts('NumMultiply')).toEqual(['Num *']);
  expect(chordParts('Ctrl+VK186')).toEqual(['Ctrl', 'VK186']);
});

test('isBrowserAccelerator: ricarica e stampa di WebView2 si inghiottono sempre', () => {
  for (const chord of ['F5', 'Ctrl+R', 'Ctrl+Shift+R', 'Ctrl+F5', 'Shift+F5', 'Ctrl+P']) expect(isBrowserAccelerator(chord), chord).toBe(true);
  for (const chord of ['F4', 'F6', 'R', 'Ctrl+S', 'Ctrl+Shift+P', 'Alt+F5', 'Ctrl+Oem3']) expect(isBrowserAccelerator(chord), chord).toBe(false);
});
