// Colonne della vista dettagliata: catalogo, configurazione salvata nelle impostazioni e griglia CSS.

import type { SortKey } from "../state/tabView";
import { splitName } from "./format";

export type ColumnId = "name" | "ext" | "size" | "modified" | "created" | "accessed" | "attributes" | "type";

/** `width` è null solo per il Nome, che prende lo spazio rimanente. */
export interface ColumnConfig {
  id: ColumnId;
  visible: boolean;
  width: number | null;
}

interface ColumnDef {
  width: number | null;
  visible: boolean;
  align: "left" | "right";
  sortKey: SortKey | null;
}

/** L'ordine delle chiavi è l'ordine di default. */
export const COLUMN_DEFS: Record<ColumnId, ColumnDef> = {
  name: { width: null, visible: true, align: "left", sortKey: "name" },
  ext: { width: 52, visible: true, align: "left", sortKey: "ext" },
  size: { width: 72, visible: true, align: "right", sortKey: "size" },
  modified: { width: 112, visible: true, align: "right", sortKey: "date" },
  created: { width: 112, visible: false, align: "right", sortKey: "created" },
  accessed: { width: 112, visible: false, align: "right", sortKey: "accessed" },
  attributes: { width: 36, visible: true, align: "right", sortKey: null },
  type: { width: 130, visible: false, align: "left", sortKey: "type" },
};

export const MIN_COL_WIDTH = 28;
const ICON_WIDTH = 16;
const MIN_NAME_WIDTH = 80;

const ids = Object.keys(COLUMN_DEFS) as ColumnId[];
const isId = (v: unknown): v is ColumnId => typeof v === "string" && Object.prototype.hasOwnProperty.call(COLUMN_DEFS, v);
const defaultOf = (id: ColumnId): ColumnConfig => ({ id, visible: COLUMN_DEFS[id].visible, width: COLUMN_DEFS[id].width });

export const DEFAULT_COLUMNS: ColumnConfig[] = ids.map(defaultOf);

/** Configurazione letta dalle impostazioni, anche di un'altra versione: sempre valida e con tutte le colonne del catalogo. */
export function normalizeColumns(raw: unknown): ColumnConfig[] {
  if (!Array.isArray(raw)) return DEFAULT_COLUMNS;
  const out: ColumnConfig[] = [];
  for (const item of raw) {
    const id = (item as { id?: unknown } | null)?.id;
    if (!isId(id) || out.some((c) => c.id === id)) continue;
    const { visible, width } = item as { visible?: unknown; width?: unknown };
    const def = defaultOf(id);
    if (id === "name") {
      out.push(def);
      continue;
    }
    out.push({
      id,
      visible: typeof visible === "boolean" ? visible : def.visible,
      width: Math.max(MIN_COL_WIDTH, typeof width === "number" && Number.isFinite(width) ? Math.round(width) : def.width!),
    });
  }
  for (const id of ids) if (!out.some((c) => c.id === id)) out.push(defaultOf(id));
  return out;
}

export const visibleColumns = (cols: ColumnConfig[], extInName: boolean): ColumnConfig[] =>
  cols.filter((c) => c.visible && !(extInName && c.id === "ext"));

/** `grid-template-columns` di intestazione e righe: icona fissa in testa, poi le colonne visibili. */
export const gridTemplate = (visible: ColumnConfig[]): string =>
  [`${ICON_WIDTH}px`, ...visible.map((c) => (c.width === null ? `minmax(${MIN_NAME_WIDTH}px, 1fr)` : `${c.width}px`))].join(" ");

export const toggleColumn = (cols: ColumnConfig[], id: ColumnId): ColumnConfig[] =>
  id === "name" ? cols : cols.map((c) => (c.id === id ? { ...c, visible: !c.visible } : c));

/** Sposta `id` davanti a `beforeId` (null = in fondo), sull'elenco completo, nascoste comprese. */
export function moveColumn(cols: ColumnConfig[], id: ColumnId, beforeId: ColumnId | null): ColumnConfig[] {
  const moved = cols.find((c) => c.id === id);
  if (!moved || id === beforeId) return cols;
  const rest = cols.filter((c) => c.id !== id);
  const at = beforeId === null ? -1 : rest.findIndex((c) => c.id === beforeId);
  return at < 0 ? [...rest, moved] : [...rest.slice(0, at), moved, ...rest.slice(at)];
}

export const resizeColumn = (cols: ColumnConfig[], id: ColumnId, width: number): ColumnConfig[] =>
  id === "name" ? cols : cols.map((c) => (c.id === id ? { ...c, width: Math.max(MIN_COL_WIDTH, Math.round(width)) } : c));

/** Chiave della cache dei tipi: `"/"` per le cartelle, altrimenti l'estensione in minuscolo (`""` se non c'è). */
export const typeKey = (name: string, isDir: boolean): string => (isDir ? "/" : splitName(name, false)[1].toLowerCase());

/**
 * `grid-column` dell'etichetta di un'unità (colonna 1 = icona): le colonne dopo il Nome, o quelle prima se il Nome
 * è in fondo; null se il Nome è l'unica colonna.
 */
export function driveSpan(visibleIds: ColumnId[]): string | null {
  const nameCol = visibleIds.indexOf("name") + 2;
  const last = visibleIds.length + 1;
  if (nameCol < last) return `${nameCol + 1} / -1`;
  return nameCol > 2 ? `2 / ${nameCol}` : null;
}
