import { create } from "zustand";

export interface Tab { id: string; path: string }
export interface Group { id: string; tabIds: string[]; activeTabId: string }
export interface LayoutState {
  groups: Group[];
  tabs: Record<string, Tab>;
  activeGroupId: string;
  /** Ogni id di gruppo una sola volta, il più recente per primo. */
  activationOrder: string[];
}

const uid = () => crypto.randomUUID();
const groupOf = (s: LayoutState, tabId: string) => s.groups.find((g) => g.tabIds.includes(tabId));
const replaceGroup = (s: LayoutState, g: Group): Group[] => s.groups.map((x) => (x.id === g.id ? g : x));

function makeGroup(path: string): { group: Group; tab: Tab } {
  const tab = { id: uid(), path };
  return { group: { id: uid(), tabIds: [tab.id], activeTabId: tab.id }, tab };
}

export function initialLayout(paths: string[]): LayoutState {
  const made = (paths.length ? paths : ["C:\\"]).map(makeGroup);
  const groups = made.map((m) => m.group);
  return {
    groups,
    tabs: Object.fromEntries(made.map((m) => [m.tab.id, m.tab])),
    activeGroupId: groups[0].id,
    activationOrder: groups.map((g) => g.id),
  };
}

export function activateGroup(s: LayoutState, groupId: string): LayoutState {
  if (!s.groups.some((g) => g.id === groupId)) return s;
  return { ...s, activeGroupId: groupId, activationOrder: [groupId, ...s.activationOrder.filter((id) => id !== groupId)] };
}

export function newTab(s: LayoutState, groupId: string, path: string): LayoutState {
  const g = s.groups.find((x) => x.id === groupId);
  if (!g) return s;
  const tab = { id: uid(), path };
  const at = g.tabIds.indexOf(g.activeTabId) + 1;
  const tabIds = [...g.tabIds.slice(0, at), tab.id, ...g.tabIds.slice(at)];
  return activateGroup(
    { ...s, tabs: { ...s.tabs, [tab.id]: tab }, groups: replaceGroup(s, { ...g, tabIds, activeTabId: tab.id }) },
    groupId,
  );
}

/** Toglie la scheda dal suo gruppo (non da `tabs`); un gruppo rimasto vuoto sparisce. */
function detachTab(s: LayoutState, tabId: string): LayoutState {
  const g = groupOf(s, tabId)!;
  const idx = g.tabIds.indexOf(tabId);
  const tabIds = g.tabIds.filter((id) => id !== tabId);
  if (tabIds.length) {
    // Vicino a destra, altrimenti a sinistra (come VS Code).
    const activeTabId = g.activeTabId === tabId ? tabIds[Math.min(idx, tabIds.length - 1)] : g.activeTabId;
    return { ...s, groups: replaceGroup(s, { ...g, tabIds, activeTabId }) };
  }
  const activationOrder = s.activationOrder.filter((id) => id !== g.id);
  return {
    ...s,
    groups: s.groups.filter((x) => x.id !== g.id),
    activationOrder,
    activeGroupId: s.activeGroupId === g.id ? activationOrder[0] : s.activeGroupId,
  };
}

export function closeTab(s: LayoutState, tabId: string): LayoutState {
  const g = groupOf(s, tabId);
  if (!g) return s;
  if (g.tabIds.length === 1 && s.groups.length === 1) return s; // mai l'ultimo gruppo
  const { [tabId]: _removed, ...tabs } = s.tabs;
  return { ...detachTab(s, tabId), tabs };
}

export function activateTab(s: LayoutState, tabId: string): LayoutState {
  const g = groupOf(s, tabId);
  if (!g) return s;
  return activateGroup({ ...s, groups: replaceGroup(s, { ...g, activeTabId: tabId }) }, g.id);
}

export function cycleTab(s: LayoutState, groupId: string, delta: 1 | -1): LayoutState {
  const g = s.groups.find((x) => x.id === groupId);
  if (!g) return s;
  const n = g.tabIds.length;
  const next = g.tabIds[(g.tabIds.indexOf(g.activeTabId) + delta + n) % n];
  return { ...s, groups: replaceGroup(s, { ...g, activeTabId: next }) };
}

export function splitGroup(s: LayoutState, groupId: string): LayoutState {
  const i = s.groups.findIndex((g) => g.id === groupId);
  if (i < 0) return s;
  const { group, tab } = makeGroup(s.tabs[s.groups[i].activeTabId].path);
  const groups = [...s.groups.slice(0, i + 1), group, ...s.groups.slice(i + 1)];
  return activateGroup({ ...s, groups, tabs: { ...s.tabs, [tab.id]: tab } }, group.id);
}

export function moveTab(s: LayoutState, tabId: string, toGroupId: string, index: number): LayoutState {
  const from = groupOf(s, tabId);
  const to = s.groups.find((g) => g.id === toGroupId);
  if (!from || !to) return s;
  const insert = (tabIds: string[]) => {
    const at = Math.min(Math.max(index, 0), tabIds.length);
    return [...tabIds.slice(0, at), tabId, ...tabIds.slice(at)];
  };
  if (from.id === to.id) {
    return { ...s, groups: replaceGroup(s, { ...from, tabIds: insert(from.tabIds.filter((id) => id !== tabId)) }) };
  }
  const detached = detachTab(s, tabId);
  const target = detached.groups.find((g) => g.id === toGroupId)!;
  return activateGroup(
    { ...detached, groups: replaceGroup(detached, { ...target, tabIds: insert(target.tabIds), activeTabId: tabId }) },
    toGroupId,
  );
}

export function setTabPath(s: LayoutState, tabId: string, path: string): LayoutState {
  return s.tabs[tabId] ? { ...s, tabs: { ...s.tabs, [tabId]: { ...s.tabs[tabId], path } } } : s;
}

/** Destinazione di F5/F6: l'altro gruppo attivato più di recente. */
export function targetGroup(s: LayoutState): string | null {
  return s.activationOrder.find((id) => id !== s.activeGroupId) ?? null;
}

export function focusAdjacentGroup(s: LayoutState, delta: 1 | -1): LayoutState {
  const n = s.groups.length;
  const i = s.groups.findIndex((g) => g.id === s.activeGroupId);
  return activateGroup(s, s.groups[(i + delta + n) % n].id);
}

interface LayoutActions {
  activateGroup: (groupId: string) => void;
  newTab: (groupId: string, path: string) => void;
  closeTab: (tabId: string) => void;
  activateTab: (tabId: string) => void;
  cycleTab: (groupId: string, delta: 1 | -1) => void;
  splitGroup: (groupId: string) => void;
  moveTab: (tabId: string, toGroupId: string, index: number) => void;
  setTabPath: (tabId: string, path: string) => void;
  focusAdjacentGroup: (delta: 1 | -1) => void;
}

export const useLayout = create<LayoutState & LayoutActions>((set) => ({
  // Il secondo gruppo come il primo: una D: assente (o un CD vuoto) darebbe un avviso all'avvio.
  ...initialLayout(["C:\\", "C:\\"]),
  activateGroup: (groupId) => set((s) => activateGroup(s, groupId)),
  newTab: (groupId, path) => set((s) => newTab(s, groupId, path)),
  closeTab: (tabId) => set((s) => closeTab(s, tabId)),
  activateTab: (tabId) => set((s) => activateTab(s, tabId)),
  cycleTab: (groupId, delta) => set((s) => cycleTab(s, groupId, delta)),
  splitGroup: (groupId) => set((s) => splitGroup(s, groupId)),
  moveTab: (tabId, toGroupId, index) => set((s) => moveTab(s, tabId, toGroupId, index)),
  setTabPath: (tabId, path) => set((s) => setTabPath(s, tabId, path)),
  focusAdjacentGroup: (delta) => set((s) => focusAdjacentGroup(s, delta)),
}));
