// Etichette della barra dei tasti funzione, per livello di modificatori.

export interface FKeySlot { key: string; commandId: string; labelKey: string }
export interface Mods { shift: boolean; ctrl: boolean; alt: boolean }

const slot = (key: string, commandId: string, label: string): FKeySlot => ({ key, commandId, labelKey: `shell.fkeys.${label}` });

const BASE: FKeySlot[] = [
  slot("F3", "cm_List", "view"),
  slot("F4", "cm_Edit", "edit"),
  slot("F5", "cm_Copy", "copy"),
  slot("F6", "cm_RenMov", "move"),
  slot("F7", "cm_MkDir", "newFolder"),
  slot("F8", "cm_Delete", "delete"),
  slot("Alt+F4", "cm_Exit", "exit"),
];
const SHIFT: FKeySlot[] = [
  slot("F5", "cm_CopySamepanel", "copyHere"),
  slot("F6", "cm_RenameOnly", "rename"),
  slot("F8", "mc_DeletePermanent", "deletePermanent"),
];
// Fase 1: dell'Alt c'è solo l'uscita.
const ALT: FKeySlot[] = [slot("F4", "cm_Exit", "exit")];
// Come in TC: vista breve o dettagliata e ordinamento.
const CTRL: FKeySlot[] = [
  slot("F1", "cm_SrcShort", "brief"),
  slot("F2", "cm_SrcLong", "full"),
  slot("F3", "cm_SrcByName", "byName"),
  slot("F4", "cm_SrcByExt", "byExt"),
  slot("F5", "cm_SrcByDateTime", "byDate"),
  slot("F6", "cm_SrcBySize", "bySize"),
  slot("F7", "cm_SrcUnsorted", "unsorted"),
];

/** Slot del livello esatto dei modificatori premuti (altre combinazioni: nessuno), solo per i comandi registrati. */
export function fkeyLabels(mods: Mods, available: Set<string>): FKeySlot[] {
  const layer = mods.ctrl ? (mods.alt || mods.shift ? [] : CTRL) : mods.alt ? (mods.shift ? [] : ALT) : mods.shift ? SHIFT : BASE;
  return layer.filter((s) => available.has(s.commandId));
}
