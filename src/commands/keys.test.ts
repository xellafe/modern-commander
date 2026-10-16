import { expect, test } from 'vitest';
import { chordFromEvent } from './keys';

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
  expect(chordFromEvent(ev(186, { c: true }))).toBe('Ctrl+VK186');
});
