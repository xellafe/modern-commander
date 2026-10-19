// Nomi dei tasti basati sul virtual key (keyCode), non sul carattere.
const NAMED: Record<number, string> = {
  8: 'Backspace', 9: 'Tab', 13: 'Enter', 27: 'Escape', 32: 'Space',
  33: 'PageUp', 34: 'PageDown', 35: 'End', 36: 'Home',
  37: 'Left', 38: 'Up', 39: 'Right', 40: 'Down',
  45: 'Insert', 46: 'Delete', 93: 'Apps',
  106: 'NumMultiply', 107: 'NumAdd', 109: 'NumSubtract',
  188: 'Comma', 192: 'Oem3', 220: 'Oem5',
};

function keyName(code: number): string {
  if (code >= 65 && code <= 90) return String.fromCharCode(code); // A-Z
  if (code >= 48 && code <= 57) return String.fromCharCode(code); // 0-9
  if (code >= 112 && code <= 135) return `F${code - 111}`; // F1-F24
  return NAMED[code] ?? `VK${code}`;
}

const DISPLAY: Record<string, string> = {
  Oem3: '`', Oem5: '\\', Comma: ',', NumAdd: 'Num +', NumSubtract: 'Num -', NumMultiply: 'Num *',
};

/** Un chord canonico ('Ctrl+Oem3') come elenco di tasti da mostrare (['Ctrl', '`']). */
export const chordParts = (chord: string): string[] => chord.split('+').map((k) => DISPLAY[k] ?? k);

// Acceleratori del browser (WebView2) che ricaricano o stampano la pagina: ricaricarla perde editor non salvati,
// uccide i terminali e azzera il layout. Si inghiottono sempre, anche quando il chord non ha un comando eseguibile.
const BROWSER_ACCELERATORS = new Set(['F5', 'Ctrl+F5', 'Shift+F5', 'Ctrl+R', 'Ctrl+Shift+R', 'Ctrl+P']);

/** Vero per i chord (nella forma di `chordFromEvent`) che il browser userebbe per ricaricare o stampare. */
export const isBrowserAccelerator = (chord: string): boolean => BROWSER_ACCELERATORS.has(chord);

export interface KeyLike {
  keyCode: number;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  getModifierState?: (key: string) => boolean;
}

// AltGr (layout italiano: AltGr+è = '[') arriva come Ctrl+Alt: diventa "AltGr+…",
// così un carattere digitato non fa mai scattare una scorciatoia Ctrl+Alt.
const isAltGr = (e: KeyLike) => e.ctrlKey && e.altKey && e.getModifierState?.('AltGraph') === true;

export function chordFromEvent(e: KeyLike): string {
  const mods = isAltGr(e)
    ? ['AltGr', e.shiftKey && 'Shift']
    : [e.ctrlKey && 'Ctrl', e.shiftKey && 'Shift', e.altKey && 'Alt'];
  return [...mods.filter(Boolean), keyName(e.keyCode)].join('+');
}

/** Carattere per la ricerca rapida: un tasto stampabile senza modificatori, oppure Ctrl+Alt+lettera (mai AltGr). */
export function quickSearchChar(e: KeyLike & { key: string; metaKey: boolean }): string | null {
  if (e.metaKey || isAltGr(e)) return null;
  if (e.ctrlKey && e.altKey) return e.keyCode >= 65 && e.keyCode <= 90 ? String.fromCharCode(e.keyCode).toLowerCase() : null;
  if (e.ctrlKey || e.altKey || e.key.length !== 1 || e.key === ' ') return null;
  return e.key;
}
