// Barra dei tasti funzione: per il livello dei modificatori premuti, i tasti F1–F12 della mappa attiva.

export interface FKeySlot { key: string; commandId: string; labelKey: string }
export interface Mods { shift: boolean; ctrl: boolean; alt: boolean }

/** Etichette brevi della barra (`shell.fkeys.*`); gli altri comandi mostrano il loro titolo. */
const SHORT: Record<string, string> = {
  cm_RenameOnly: "rename",
  cm_List: "view",
  cm_Edit: "edit",
  cm_Copy: "copy",
  cm_RenMov: "move",
  cm_MkDir: "newFolder",
  cm_Delete: "delete",
  cm_Exit: "exit",
  cm_CopySamepanel: "copyHere",
  cm_PackFiles: "pack",
  cm_UnpackFiles: "unpack",
  mc_DeletePermanent: "deletePermanent",
  cm_SrcOpenDrives: "drives",
  cm_SrcThumbs: "thumbs",
  cm_SrcShort: "brief",
  cm_SrcLong: "full",
  cm_SrcByName: "byName",
  cm_SrcByExt: "byExt",
  cm_SrcByDateTime: "byDate",
  cm_SrcBySize: "bySize",
  cm_SrcUnsorted: "unsorted",
};

const F_KEYS = Array.from({ length: 12 }, (_, i) => `F${i + 1}`);

/**
 * Slot del livello esatto dei modificatori premuti, letti da `keymap` (chord → id comando, nell'ordine di
 * `chordFromEvent`: Ctrl, Shift, Alt). `titleKey` dà il titolo di un comando registrato; i non registrati non compaiono.
 */
export function fkeyLabels(
  mods: Mods,
  keymap: ReadonlyMap<string, string>,
  titleKey: (commandId: string) => string | undefined,
): FKeySlot[] {
  const prefix = [mods.ctrl && "Ctrl", mods.shift && "Shift", mods.alt && "Alt"].filter(Boolean).join("+");
  const chords = F_KEYS.map((k) => (prefix ? `${prefix}+${k}` : k));
  // Senza modificatori, come in TC, in fondo anche Alt+F4 (l'uscita).
  if (!prefix) chords.push("Alt+F4");
  return chords.flatMap((chord) => {
    const id = keymap.get(chord);
    const title = id === undefined ? undefined : titleKey(id);
    if (id === undefined || title === undefined) return [];
    const key = prefix ? chord.slice(prefix.length + 1) : chord;
    return [{ key, commandId: id, labelKey: SHORT[id] ? `shell.fkeys.${SHORT[id]}` : title }];
  });
}
