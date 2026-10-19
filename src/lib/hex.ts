// Righe della vista esadecimale: 16 byte per riga, colonne sempre allineate.

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).toUpperCase().padStart(2, "0"));

/** Cifre dell'offset: almeno 8, di più se la dimensione non ci sta. */
export const offsetDigits = (size: number): number => Math.max(8, size.toString(16).length);

export function hexRow(bytes: Uint8Array, offset: number, digits: number): { offset: string; hex: string; ascii: string } {
  let hex = "";
  let ascii = "";
  for (let i = 0; i < 16; i++) {
    hex += i === 8 ? "  " : i ? " " : "";
    if (i < bytes.length) {
      const b = bytes[i];
      hex += HEX[b];
      ascii += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : ".";
    } else hex += "  ";
  }
  return { offset: offset.toString(16).toUpperCase().padStart(digits, "0"), hex, ascii };
}
