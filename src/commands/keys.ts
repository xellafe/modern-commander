// Nomi dei tasti basati sul virtual key (keyCode), non sul carattere.
const NAMED: Record<number, string> = {
  8: 'Backspace', 9: 'Tab', 13: 'Enter', 27: 'Escape', 32: 'Space',
  33: 'PageUp', 34: 'PageDown', 35: 'End', 36: 'Home',
  37: 'Left', 38: 'Up', 39: 'Right', 40: 'Down',
  45: 'Insert', 46: 'Delete',
  106: 'NumMultiply', 107: 'NumAdd', 109: 'NumSubtract',
  188: 'Comma', 192: 'Oem3', 220: 'Oem5',
};

function keyName(code: number): string {
  if (code >= 65 && code <= 90) return String.fromCharCode(code); // A-Z
  if (code >= 48 && code <= 57) return String.fromCharCode(code); // 0-9
  if (code >= 112 && code <= 135) return `F${code - 111}`; // F1-F24
  return NAMED[code] ?? `VK${code}`;
}

export function chordFromEvent(e: { keyCode: number; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): string {
  const mods = [e.ctrlKey && 'Ctrl', e.shiftKey && 'Shift', e.altKey && 'Alt'].filter(Boolean);
  return [...mods, keyName(e.keyCode)].join('+');
}
