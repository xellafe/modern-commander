// Codifiche del testo: rilevamento e unità di codice. Le decodifica `TextDecoder` (etichette WHATWG).

export interface Detected {
  encoding: string;
  /** Lunghezza in byte del BOM (0, 2, 3). */
  bom: number;
}

/** Etichette proposte nel viewer. Le codifiche DOS (437/850) non sono WHATWG e restano fuori. */
export const ENCODINGS = [
  "utf-8", "utf-16le", "utf-16be",
  "windows-1250", "windows-1251", "windows-1252", "windows-1253", "windows-1254",
  "windows-1255", "windows-1256", "windows-1257", "windows-1258",
  "iso-8859-2", "iso-8859-15", "koi8-r", "shift_jis", "gbk", "big5", "euc-kr",
];

/** `complete`: il campione è il file intero. Se falso una sequenza UTF-8 troncata in fondo non è un errore. */
export function detectEncoding(sample: Uint8Array, ansi: string, complete: boolean): Detected {
  const [a, b, c] = sample;
  if (a === 0xef && b === 0xbb && c === 0xbf) return { encoding: "utf-8", bom: 3 };
  if (a === 0xff && b === 0xfe) return { encoding: "utf-16le", bom: 2 };
  if (a === 0xfe && b === 0xff) return { encoding: "utf-16be", bom: 2 };
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(sample, { stream: !complete });
    return { encoding: "utf-8", bom: 0 };
  } catch {
    return { encoding: ansi, bom: 0 };
  }
}

/** Byte per unità di codice: 2 solo per UTF-16. */
export const codeUnit = (encoding: string): 1 | 2 => (encoding === "utf-16le" || encoding === "utf-16be" ? 2 : 1);
