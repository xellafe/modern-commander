// Formattazione delle colonne della lista file.

const ATTRS: [number, string][] = [[0x1, "r"], [0x20, "a"], [0x2, "h"], [0x4, "s"]];

/** Attributi Win32 nello stile di TC: `rahs`, `-` dove manca. */
export const formatAttrs = (a: number): string => ATTRS.map(([bit, c]) => (a & bit ? c : "-")).join("");

/** [nome, estensione]; le cartelle e i nomi che iniziano col punto non hanno estensione. */
export function splitName(name: string, isDir: boolean): [string, string] {
  const i = name.lastIndexOf(".");
  return isDir || i <= 0 ? [name, ""] : [name.slice(0, i), name.slice(i + 1)];
}

export type FileKind =
  | "folder" | "image" | "code" | "data" | "doc" | "pdf" | "archive"
  | "audio" | "video" | "sheet" | "exec" | "file";

const KINDS: Record<string, FileKind> = {};
const add = (kind: FileKind, exts: string) => exts.split(" ").forEach((e) => (KINDS[e] = kind));
add("image", "png jpg jpeg gif bmp webp svg ico tif tiff heic avif");
add("code", "ts tsx js jsx mjs cjs rs py cs java c h cpp hpp go rb php html css scss sql sh lua kt swift vue");
add("data", "json jsonc toml yaml yml xml ini cfg conf lock env");
add("doc", "txt md log rtf doc docx odt");
add("pdf", "pdf");
add("archive", "zip 7z rar tar gz tgz bz2 xz cab iso");
add("audio", "mp3 wav flac ogg m4a aac wma");
add("video", "mp4 mkv avi mov wmv webm m4v");
add("sheet", "xls xlsx csv ods");
add("exec", "exe msi bat cmd ps1 com lnk dll");

export const fileKind = (name: string, isDir: boolean): FileKind =>
  isDir ? "folder" : (KINDS[splitName(name, false)[1].toLowerCase()] ?? "file");

// Simboli uguali in tutte le lingue (Intl darebbe "byte" e "kB"); il numero segue la lingua.
const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;
const sizeFormats = new Map<string, Intl.NumberFormat[]>();

/** Dimensione leggibile in base 1024: `1,5 KB` in italiano, `1.5 KB` in inglese. */
export function formatSize(bytes: number, lang: string): string {
  let fmts = sizeFormats.get(lang);
  if (!fmts) {
    fmts = UNITS.map((_, i) => new Intl.NumberFormat(lang, { maximumFractionDigits: i ? 1 : 0 }));
    sizeFormats.set(lang, fmts);
  }
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < UNITS.length - 1) { n /= 1024; i++; }
  return `${fmts[i].format(n)} ${UNITS[i]}`;
}

/** Tempo rimanente: `m:ss`, oppure `h:mm:ss` da un'ora in su. */
export function formatEta(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  const [h, m] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60)];
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

const dateFormats = new Map<string, Intl.DateTimeFormat>();

export function formatDate(ms: number, locale: string): string {
  let f = dateFormats.get(locale);
  if (!f) dateFormats.set(locale, (f = new Intl.DateTimeFormat(locale, { dateStyle: "short", timeStyle: "short" })));
  return f.format(ms);
}
