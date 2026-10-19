import { expect, test } from 'vitest';
import { fuzzyFilter, fuzzyMatch, highlightRuns } from './fuzzy';

test('fuzzyMatch', () => {
  expect(fuzzyMatch('cpy', 'Copy')!.positions).toEqual([0, 2, 3]);
  expect(fuzzyMatch('xyz', 'Copy')).toBeNull();
  expect(fuzzyMatch('fl', 'FileList.tsx')!.positions).toEqual([0, 4]);
});

test('fuzzyMatch scores: +1 per char, +8 word start, +5 consecutive, +10 at position 0', () => {
  expect(fuzzyMatch('fl', 'FileList.tsx')!.score).toBe(19 + 9); // F: 1+8+10, L: 1+8
  expect(fuzzyMatch('cpy', 'Copy')!.score).toBe(19 + 1 + 6); // C: 1+8+10, p: 1, y: 1+5
  expect(fuzzyMatch('src', 'my_src_x')!.score).toBe(9 + 6 + 6);
});

test('fuzzyMatch is case-insensitive and word starts include separators and camelCase', () => {
  expect(fuzzyMatch('CP', 'copy')!.positions).toEqual([0, 2]);
  expect(fuzzyMatch('ab', 'x\\a-b')!.positions).toEqual([2, 4]);
  expect(fuzzyMatch('ab', 'x/a.b')!.positions).toEqual([2, 4]);
  expect(fuzzyMatch('mc', 'myCar')!.positions).toEqual([0, 2]);
});

test('fuzzyMatch falls back when a word start would leave the rest unmatched', () => {
  // la 'a' a inizio di parola (indice 4) non lascia una 'b' dopo di sé: si ripiega sulla sottosequenza semplice
  expect(fuzzyMatch('ab', 'xab_a')!.positions).toEqual([1, 2]);
  expect(fuzzyMatch('ba', 'ab')).toBeNull();
});

test('fuzzyMatch with an empty query matches with no positions', () => {
  expect(fuzzyMatch('', 'abc')).toEqual({ score: 0, positions: [] });
});

test('fuzzyFilter ranks prefixes and word starts first', () => {
  expect(fuzzyFilter(['resources', 'my_src_x', 'src'], 'src', (s) => s, 10).map((r) => r.item)).toEqual(['src', 'my_src_x', 'resources']);
  expect(fuzzyFilter(['b', 'a'], '', (s) => s, 1).map((r) => r.item)).toEqual(['b']);
});

test('fuzzyFilter is stable for ties, drops non-matches and honors the limit', () => {
  const items = ['x1', 'x2', 'zz', 'x3'];
  expect(fuzzyFilter(items, 'x', (s) => s, 10).map((r) => r.item)).toEqual(['x1', 'x2', 'x3']);
  expect(fuzzyFilter(items, 'x', (s) => s, 2).map((r) => r.item)).toEqual(['x1', 'x2']);
});

test('highlightRuns splits text into matched and unmatched runs', () => {
  expect(highlightRuns('Copy', [0, 2, 3])).toEqual([
    { text: 'C', hit: true },
    { text: 'o', hit: false },
    { text: 'py', hit: true },
  ]);
  expect(highlightRuns('abc', [])).toEqual([{ text: 'abc', hit: false }]);
  expect(highlightRuns('', [])).toEqual([]);
});
