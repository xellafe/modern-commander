import { create } from "zustand";
import { readConfig, writeConfig } from "../ipc/api";

export type HotlistItem =
  | { type: "dir"; id: string; name: string; path: string }
  | { type: "submenu"; id: string; name: string; items: HotlistItem[] }
  | { type: "separator"; id: string };

const uid = () => crypto.randomUUID();

function find(list: HotlistItem[], id: string): HotlistItem | undefined {
  for (const i of list) {
    if (i.id === id) return i;
    const inner = i.type === "submenu" ? find(i.items, id) : undefined;
    if (inner) return inner;
  }
}

/** `id` è dentro il sottomenu `ancestorId` (a qualsiasi profondità, escluso il sottomenu stesso). */
export function isInside(list: HotlistItem[], ancestorId: string, id: string): boolean {
  const a = find(list, ancestorId);
  return a?.type === "submenu" && find(a.items, id) !== undefined;
}

/** Genitore (null = radice) e posizione di un elemento. */
function locate(list: HotlistItem[], id: string, parentId: string | null = null): { parentId: string | null; index: number } | null {
  const index = list.findIndex((i) => i.id === id);
  if (index >= 0) return { parentId, index };
  for (const i of list) {
    const r = i.type === "submenu" ? locate(i.items, id, i.id) : null;
    if (r) return r;
  }
  return null;
}

/** Inserisce in `parentId` (null = radice); null se il genitore non esiste o non è un sottomenu. */
function insertAt(list: HotlistItem[], parentId: string | null, index: number, item: HotlistItem): HotlistItem[] | null {
  if (parentId === null) {
    const at = Math.min(Math.max(index, 0), list.length);
    return [...list.slice(0, at), item, ...list.slice(at)];
  }
  let done = false;
  const out = list.map((i) => {
    if (done || i.type !== "submenu") return i;
    const items = insertAt(i.items, i.id === parentId ? null : parentId, index, item);
    if (!items) return i;
    done = true;
    return { ...i, items };
  });
  return done ? out : null;
}

export function addDir(list: HotlistItem[], path: string, name: string, parentId?: string): HotlistItem[] {
  const item: HotlistItem = { type: "dir", id: uid(), name, path };
  return insertAt(list, parentId ?? null, Number.MAX_SAFE_INTEGER, item) ?? list;
}

export function addSubmenu(list: HotlistItem[], name: string, parentId?: string, id: string = uid()): HotlistItem[] {
  const item: HotlistItem = { type: "submenu", id, name, items: [] };
  return insertAt(list, parentId ?? null, Number.MAX_SAFE_INTEGER, item) ?? list;
}

export function removeItem(list: HotlistItem[], id: string): HotlistItem[] {
  return list
    .filter((i) => i.id !== id)
    .map((i) => (i.type === "submenu" ? { ...i, items: removeItem(i.items, id) } : i));
}

export function renameItem(list: HotlistItem[], id: string, name: string): HotlistItem[] {
  return list.map((i) => {
    if (i.type === "separator") return i;
    if (i.id === id) return { ...i, name };
    return i.type === "submenu" ? { ...i, items: renameItem(i.items, id, name) } : i;
  });
}

/** `index` conta nella lista di destinazione senza l'elemento spostato. Mosse non valide: lista invariata. */
export function moveItem(list: HotlistItem[], id: string, toParentId: string | null, index: number): HotlistItem[] {
  const item = find(list, id);
  if (!item) return list;
  if (toParentId !== null && (toParentId === id || isInside(list, id, toParentId) || find(list, toParentId)?.type !== "submenu")) {
    return list;
  }
  return insertAt(removeItem(list, id), toParentId, index, item) ?? list;
}

/** Traduce un rilascio (prima/dopo/dentro `targetId`) in genitore e indice per `moveItem`; null se non valido. */
export function resolveDrop(
  list: HotlistItem[],
  dragId: string,
  targetId: string,
  where: "before" | "after" | "into",
): { toParentId: string | null; index: number } | null {
  const target = find(list, targetId);
  if (!target || dragId === targetId || isInside(list, dragId, targetId)) return null;
  if (where === "into") {
    return target.type === "submenu" ? { toParentId: target.id, index: target.items.filter((i) => i.id !== dragId).length } : null;
  }
  const at = locate(list, targetId)!;
  const from = locate(list, dragId);
  const index = at.index + (where === "after" ? 1 : 0);
  return { toParentId: at.parentId, index: from && from.parentId === at.parentId && from.index < index ? index - 1 : index };
}

const isStr = (v: unknown): v is string => typeof v === "string";

/** Valida il contenuto di hotlist.json: una lista di voci ben formate con id unici, altrimenti null. */
export function parseHotlist(raw: unknown): HotlistItem[] | null {
  const seen = new Set<string>();
  const ok = (v: unknown): boolean => {
    if (typeof v !== "object" || v === null) return false;
    const i = v as Record<string, unknown>;
    if (!isStr(i.id) || seen.has(i.id)) return false;
    seen.add(i.id);
    if (i.type === "separator") return true;
    if (!isStr(i.name)) return false;
    if (i.type === "dir") return isStr(i.path);
    return i.type === "submenu" && Array.isArray(i.items) && i.items.every(ok);
  };
  return Array.isArray(raw) && raw.every(ok) ? (raw as HotlistItem[]) : null;
}

/** Le cartelle dei Preferiti, anche dentro i sottomenu, nell'ordine in cui compaiono nel menu (separatori esclusi). */
export function hotlistDirs(items: HotlistItem[]): { name: string; path: string }[] {
  return items.flatMap((i) => (i.type === "dir" ? [{ name: i.name, path: i.path }] : i.type === "submenu" ? hotlistDirs(i.items) : []));
}

/** Dove aprire il menu: all'angolo (riga col cursore) o centrato (nessuna riga visibile). */
export interface MenuAnchor {
  x: number;
  y: number;
  center: boolean;
  /** Cartella della scheda attiva al momento dell'apertura (per "Aggiungi cartella corrente"). */
  path: string;
}

interface HotlistState {
  items: HotlistItem[];
  loaded: boolean;
  /** Vista Preferiti in modifica (rinomina, elimina, trascina). */
  editing: boolean;
  menu: MenuAnchor | null;
  load: () => Promise<void>;
  addDir: (path: string, name: string, parentId?: string) => void;
  /** Restituisce l'id del nuovo sottomenu. */
  addSubmenu: (name: string, parentId?: string) => string;
  remove: (id: string) => void;
  rename: (id: string, name: string) => void;
  move: (id: string, toParentId: string | null, index: number) => void;
  setEditing: (editing: boolean) => void;
  openMenu: (anchor: MenuAnchor) => void;
  closeMenu: () => void;
}

// Le scritture partono in ordine di chiamata: l'ultima modifica vince sempre.
let pending: Promise<unknown> = Promise.resolve();

export const useHotlist = create<HotlistState>((set, get) => {
  // Una modifica che non cambia niente (mossa non valida) non riscrive il file.
  const change = (fn: (l: HotlistItem[]) => HotlistItem[]) => {
    const prev = get().items;
    const items = fn(prev);
    if (items === prev) return;
    set({ items });
    const run = pending.then(() => writeConfig("hotlist", items));
    pending = run.catch(() => {}); // un errore non blocca le scritture successive
    run.catch(console.error);
  };

  return {
    items: [],
    loaded: false,
    editing: false,
    menu: null,
    // File mancante, illeggibile o malformato: lista vuota, e il file resta com'è finché l'utente non cambia qualcosa.
    load: async () => {
      const items = await readConfig("hotlist").then(parseHotlist, () => null);
      set({ items: items ?? [], loaded: true });
    },
    addDir: (path, name, parentId) => change((l) => addDir(l, path, name, parentId)),
    addSubmenu: (name, parentId) => {
      const id = uid();
      change((l) => addSubmenu(l, name, parentId, id));
      return id;
    },
    remove: (id) => change((l) => removeItem(l, id)),
    rename: (id, name) => change((l) => renameItem(l, id, name)),
    move: (id, toParentId, index) => change((l) => moveItem(l, id, toParentId, index)),
    setEditing: (editing) => set({ editing }),
    openMenu: (menu) => set({ menu }),
    closeMenu: () => set({ menu: null }),
  };
});
