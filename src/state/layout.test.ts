import { expect, test } from 'vitest';
import {
  activateGroup, activateTab, closeTab, cycleTab, focusAdjacentGroup, initialLayout,
  moveTab, newTab, setTabPath, splitGroup, targetGroup, useLayout,
} from './layout';

test('the default layout opens two groups on C:\\, never on a drive that may be missing', () => {
  const { groups, tabs } = useLayout.getState();
  expect(groups.map((g) => tabs[g.activeTabId].path)).toEqual(['C:\\', 'C:\\']);
});

test('targetGroup is most recently active other group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = splitGroup(s, s.groups[1].id); // 3 gruppi, attivo il terzo
  const [g1, g2, g3] = s.groups.map((g) => g.id);
  s = activateGroup(activateGroup(s, g1), g3);
  expect(targetGroup(s)).toBe(g1);
  s = activateGroup(s, g2);
  expect(targetGroup(s)).toBe(g3);
});

test('targetGroup is null with a single group', () => {
  expect(targetGroup(initialLayout(['C:\\']))).toBeNull();
});

test('closing last tab removes the group, but never the last group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = closeTab(s, s.groups[1].activeTabId);
  expect(s.groups).toHaveLength(1);
  const before = s;
  s = closeTab(s, s.groups[0].activeTabId);
  expect(s).toEqual(before);
});

test('moving the only tab out of a group removes that group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = moveTab(s, s.groups[1].activeTabId, s.groups[0].id, 1);
  expect(s.groups).toHaveLength(1);
  expect(s.groups[0].tabIds).toHaveLength(2);
});

test('focusAdjacentGroup wraps', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = activateGroup(s, s.groups[1].id);
  expect(focusAdjacentGroup(s, 1).activeGroupId).toBe(s.groups[0].id);
  const f = initialLayout(['C:\\', 'D:\\']);
  expect(focusAdjacentGroup(f, -1).activeGroupId).toBe(f.groups[1].id);
});

test('initialLayout: empty paths behave like C:\\, first group is active', () => {
  const e = initialLayout([]);
  expect(e.groups).toHaveLength(1);
  expect(e.tabs[e.groups[0].activeTabId].path).toBe('C:\\');
  const s = initialLayout(['C:\\', 'D:\\']);
  expect(s.activeGroupId).toBe(s.groups[0].id);
  expect(s.activationOrder).toEqual(s.groups.map((g) => g.id));
});

test('activationOrder holds each group once, newest first; split inserts right of source', () => {
  let s = initialLayout(['C:\\', 'D:\\', 'E:\\']);
  const [g1, g2, g3] = s.groups.map((g) => g.id);
  s = splitGroup(s, g1);
  const g4 = s.activeGroupId;
  expect(s.groups.map((g) => g.id)).toEqual([g1, g4, g2, g3]);
  expect(s.activationOrder).toEqual([g4, g1, g2, g3]);
  expect(s.tabs[s.groups[1].activeTabId].path).toBe('C:\\');
  s = closeTab(s, s.groups[1].activeTabId); // chiude il gruppo attivo
  expect(s.activationOrder).toEqual([g1, g2, g3]);
  expect(s.activeGroupId).toBe(g1);
});

test('closeTab of the active tab activates right neighbor, else left', () => {
  let s = initialLayout(['C:\\']);
  const gid = s.groups[0].id;
  s = newTab(s, gid, 'A'); s = newTab(s, gid, 'B');
  const [t0, t1, t2] = s.groups[0].tabIds;
  s = activateTab(s, t1);
  s = closeTab(s, t1);
  expect(s.groups[0].activeTabId).toBe(t2);
  s = closeTab(s, t2);
  expect(s.groups[0].activeTabId).toBe(t0);
  expect(s.tabs[t1]).toBeUndefined();
});

test('newTab inserts after the active tab and activates it', () => {
  let s = initialLayout(['C:\\']);
  const gid = s.groups[0].id;
  s = newTab(s, gid, 'A');
  const [t0] = s.groups[0].tabIds;
  s = activateTab(s, t0);
  s = newTab(s, gid, 'B');
  expect(s.groups[0].tabIds.map((id) => s.tabs[id].path)).toEqual(['C:\\', 'B', 'A']);
  expect(s.tabs[s.groups[0].activeTabId].path).toBe('B');
});

test('cycleTab wraps', () => {
  let s = initialLayout(['C:\\']);
  const gid = s.groups[0].id;
  s = newTab(s, gid, 'A');
  const [t0, t1] = s.groups[0].tabIds;
  expect(s.groups[0].activeTabId).toBe(t1);
  expect(cycleTab(s, gid, 1).groups[0].activeTabId).toBe(t0);
  expect(cycleTab(activateTab(s, t0), gid, -1).groups[0].activeTabId).toBe(t1);
});

test('moveTab reorders in the same group (clamped) and activates across groups', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  const [g1, g2] = s.groups.map((g) => g.id);
  s = newTab(s, g1, 'A'); s = newTab(s, g1, 'B');
  const [t0, t1, t2] = s.groups[0].tabIds;
  s = moveTab(s, t0, g1, 99);
  expect(s.groups[0].tabIds).toEqual([t1, t2, t0]);
  const d = s.groups[1].activeTabId;
  s = moveTab(s, t1, g2, 0);
  expect(s.groups[1].tabIds[0]).toBe(t1);
  expect(s.groups[1].tabIds).toContain(d);
  expect(s.groups[1].activeTabId).toBe(t1);
  expect(s.activeGroupId).toBe(g2);
  expect(s.groups[0].tabIds).toEqual([t2, t0]);
});

test('setTabPath updates the path', () => {
  const s = initialLayout(['C:\\']);
  const t = s.groups[0].activeTabId;
  expect(setTabPath(s, t, 'C:\\x').tabs[t].path).toBe('C:\\x');
});

test('unknown ids return the same state', () => {
  const s = initialLayout(['C:\\', 'D:\\']);
  expect(activateGroup(s, 'x')).toBe(s);
  expect(newTab(s, 'x', 'p')).toBe(s);
  expect(closeTab(s, 'x')).toBe(s);
  expect(activateTab(s, 'x')).toBe(s);
  expect(cycleTab(s, 'x', 1)).toBe(s);
  expect(splitGroup(s, 'x')).toBe(s);
  expect(moveTab(s, 'x', s.groups[0].id, 0)).toBe(s);
  expect(moveTab(s, s.groups[0].activeTabId, 'x', 0)).toBe(s);
  expect(setTabPath(s, 'x', 'p')).toBe(s);
});

test('useLayout starts with C:\\ and D:\\ and wraps the functions', () => {
  const st = useLayout.getState();
  expect(st.groups).toHaveLength(2);
  st.focusAdjacentGroup(1);
  expect(useLayout.getState().activeGroupId).toBe(st.groups[1].id);
});
