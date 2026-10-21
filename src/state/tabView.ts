import type { DirSize, Drive, Entry } from "../ipc/types";
import { typeKey } from "../lib/columns";
import { compileMask } from "../lib/mask";

export type SortKey = "name" | "ext" | "size" | "date" | "created" | "accessed" | "type" | "none";
export interface Row extends Entry {
  isParent?: boolean;
  /** Solo nella pseudo-cartella `drives:`: l'unità rappresentata dalla riga. */
  drive?: Drive;
}
export interface TabView {
  rows: Row[];
  cursor: number;
  marked: Set<string>;
  sort: { key: SortKey; desc: boolean };
  history: string[];
  historyIndex: number;
  filter: string;
  view: "full" | "brief" | "thumbs";
  /** Byte delle cartelle già misurate, per nome. Leggere con `hasSize`: un nome come "constructor" non è una chiave assente. */
  dirSizes: Record<string, number>;
  /** Cartelle con il calcolo della dimensione in corso. */
  sizing: Set<string>;
  /** Sottocartelle illeggibili non comprese nella dimensione (solo se > 0). */
  skipped: Record<string, number>;
}
interface BuildOpts {
  showHidden: boolean;
  showSystem: boolean;
  filter: string;
  dirSizes?: Record<string, number>;
  /** Testi dei tipi per chiave di `typeKey`: servono solo all'ordinamento per Tipo. */
  typeNames?: Record<string, string>;
}

const ATTR_HIDDEN = 0x2;
const ATTR_SYSTEM = 0x4;
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export const emptyView = (path: string): TabView => ({
  rows: [],
  cursor: 0,
  marked: new Set(),
  sort: { key: "name", desc: false },
  history: [path],
  historyIndex: 0,
  filter: "",
  view: "full",
  dirSizes: {},
  sizing: new Set(),
  skipped: {},
});

export const hasSize = (sizes: Record<string, unknown>, name: string): boolean =>
  Object.prototype.hasOwnProperty.call(sizes, name); // Object.hasOwn è ES2022, il target è ES2020

export function parentPath(path: string): string | null {
  const p = path.replace(/\\+$/, "");
  if (p.startsWith("\\\\")) {
    const parts = p.slice(2).split("\\");
    return parts.length <= 2 ? null : `\\\\${parts.slice(0, -1).join("\\")}`;
  }
  const i = p.lastIndexOf("\\");
  if (i < 0) return null; // radice dell'unità ("C:")
  const parent = p.slice(0, i);
  return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent;
}

const extOf = (name: string): string => {
  const i = name.lastIndexOf(".");
  return i <= 0 ? "" : name.slice(i + 1);
};

export function buildRows(
  entries: Entry[],
  path: string,
  sort: TabView["sort"],
  opts: BuildOpts,
): Row[] {
  const sizes = opts.dirSizes ?? {};
  const types = opts.typeNames ?? {};
  const typeOf = (e: Entry) => {
    const k = typeKey(e.name, e.isDir);
    return hasSize(types, k) ? types[k] : ""; // "a.constructor" non deve leggere il prototipo
  };
  const sizeOf = (e: Entry) => (e.isDir ? (hasSize(sizes, e.name) ? sizes[e.name] : 0) : e.size);
  const key = (a: Entry, b: Entry): number => {
    switch (sort.key) {
      case "name": return collator.compare(a.name, b.name);
      case "ext": return collator.compare(extOf(a.name), extOf(b.name));
      case "size": return sizeOf(a) - sizeOf(b);
      case "date": return a.modifiedMs - b.modifiedMs;
      case "created": return a.createdMs - b.createdMs;
      case "accessed": return a.accessedMs - b.accessedMs;
      case "type": return collator.compare(typeOf(a), typeOf(b));
      default: return 0;
    }
  };
  const match = compileMask(opts.filter);
  const visible = entries.filter(
    (e) =>
      (opts.showHidden || !(e.attributes & ATTR_HIDDEN)) &&
      (opts.showSystem || !(e.attributes & ATTR_SYSTEM)) &&
      (e.isDir || !opts.filter || match(e.name)),
  );
  // Array.sort è stabile: con "none" restano l'ordine del listing dentro ogni blocco.
  const sorted = visible.sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    const k = key(a, b);
    if (k !== 0) return sort.desc ? -k : k;
    return sort.key === "none" ? 0 : collator.compare(a.name, b.name);
  });
  const parent: Row[] =
    parentPath(path) === null
      ? []
      : [{ name: "..", isDir: true, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false, isParent: true }];
  return [...parent, ...sorted];
}

export function toggleMark(v: TabView, name: string): TabView {
  if (name === "..") return v;
  const marked = new Set(v.marked);
  if (!marked.delete(name)) marked.add(name);
  return { ...v, marked };
}

export function markRange(v: TabView, from: number, to: number): TabView {
  const marked = new Set(v.marked);
  for (const r of v.rows.slice(Math.min(from, to), Math.max(from, to) + 1)) {
    if (!r.isParent) marked.add(r.name);
  }
  return { ...v, marked };
}

export function selectByMask(v: TabView, mask: string, select: boolean): TabView {
  const include = mask.split("|")[0].trim();
  const withDirs = include === "*" || include === "*.*";
  const marked = new Set(v.marked);
  const match = compileMask(mask);
  for (const r of v.rows) {
    if (r.isParent || (r.isDir && !withDirs) || !match(r.name)) continue;
    if (select) marked.add(r.name);
    else marked.delete(r.name);
  }
  return { ...v, marked };
}

// ponytail: inverte file e cartelle (esclusa '..'); se TC invertisse solo i file, filtrare qui.
export function invertMarks(v: TabView): TabView {
  const marked = new Set<string>();
  for (const r of v.rows) {
    if (!r.isParent && !v.marked.has(r.name)) marked.add(r.name);
  }
  return { ...v, marked };
}

/** Cartelle di cui ha senso calcolare la dimensione: non '..' né le unità di `drives:`. */
export const sizable = (r: Row): boolean => r.isDir && !r.isParent && !r.drive;

export const startSizing = (v: TabView, names: string[]): TabView => ({
  ...v,
  sizing: new Set([...v.sizing, ...names]),
});

/** Chiude il calcolo di `name`; `size` null (annullato o fallito) lascia invariata la dimensione nota. */
export function finishSizing(v: TabView, name: string, size: DirSize | null): TabView {
  const sizing = new Set(v.sizing);
  sizing.delete(name);
  if (!size) return { ...v, sizing };
  const { [name]: _old, ...skipped } = v.skipped;
  return {
    ...v,
    sizing,
    dirSizes: { ...v.dirSizes, [name]: size.bytes },
    skipped: size.skipped > 0 ? { ...skipped, [name]: size.skipped } : skipped,
  };
}

const moved = (v: TabView, history: string[], historyIndex: number): TabView => ({
  ...v,
  history,
  historyIndex,
  cursor: 0,
  marked: new Set(),
  filter: "",
  dirSizes: {},
  sizing: new Set(),
  skipped: {},
});

export function navigateTo(v: TabView, path: string): TabView {
  if (v.history[v.historyIndex] === path) return v;
  const history = [...v.history.slice(0, v.historyIndex + 1), path];
  return moved(v, history, history.length - 1);
}

export const back = (v: TabView): TabView =>
  v.historyIndex <= 0 ? v : moved(v, v.history, v.historyIndex - 1);

export const forward = (v: TabView): TabView =>
  v.historyIndex >= v.history.length - 1 ? v : moved(v, v.history, v.historyIndex + 1);

/** Salto a una voce qualsiasi della history (menu Alt+↓). */
export const jumpTo = (v: TabView, index: number): TabView =>
  index < 0 || index >= v.history.length || index === v.historyIndex ? v : moved(v, v.history, index);

export function quickSearch(rows: Row[], prefix: string, from: number): number {
  if (!prefix) return -1;
  const p = prefix.toLowerCase();
  for (let n = 0; n < rows.length; n++) {
    const i = (from + n) % rows.length;
    if (!rows[i].isParent && rows[i].name.toLowerCase().startsWith(p)) return i;
  }
  return -1;
}
