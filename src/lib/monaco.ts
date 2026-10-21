// Caricamento pigro di Monaco: il pacchetto e i suoi worker restano in chunk separati, fuori da quello principale.
import type * as Monaco from "monaco-editor";

type MonacoApi = typeof Monaco;

/** Il linguaggio (id Monaco) dell'ultima estensione del nome; null se nessuno la dichiara. */
export function languageFor(name: string, languages: { id: string; extensions?: string[] }[]): string | null {
  const dot = name.lastIndexOf(".");
  if (dot < 0) return null;
  const ext = name.slice(dot).toLowerCase();
  return languages.find((l) => l.extensions?.includes(ext))?.id ?? null;
}

/**
 * Se conviene colorare con `colorize`: non per il testo semplice (`plaintext`, che ha l'estensione `.txt`) né per
 * righe oltre `MAX_COLORIZE_LINE` (minificati). `colorize` costa più che lineare nelle righe e gira sul thread della UI.
 */
export function worthColorizing(text: string, language: string | null): language is string {
  if (!language || language === "plaintext") return false;
  for (let start = 0; start < text.length; ) {
    const nl = text.indexOf("\n", start);
    const end = nl < 0 ? text.length : nl;
    if (end - start > MAX_COLORIZE_LINE) return false;
    start = end + 1;
  }
  return true;
}
const MAX_COLORIZE_LINE = 2000;

/** Righe colorate nell'anteprima: oltre, `colorize` blocca la UI per centinaia di ms (256 KiB a righe corte). */
export const COLORIZE_LINES = 2000;

/** Le prime `lines` righe (con il loro a capo) da colorare, e il resto da mostrare come testo semplice. */
export function splitForColorize(text: string, lines = COLORIZE_LINES): [string, string] {
  let at = 0;
  for (let n = 0; n < lines; n++) {
    const nl = text.indexOf("\n", at);
    if (nl < 0) return [text, ""];
    at = nl + 1;
  }
  return [text.slice(0, at), text.slice(at)];
}

/** Colore CSS dei token (`#rgb`, `#rrggbb`, `rgb()`, `rgba()`) in esadecimale, l'unico formato dei temi di Monaco. */
export function cssToHex(value: string): string | undefined {
  const v = value.trim().toLowerCase();
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(v);
  if (short) return `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}`;
  if (/^#([0-9a-f]{6}|[0-9a-f]{8})$/.test(v)) return v;
  const fn = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(v);
  if (!fn) return undefined;
  const hex = (n: number) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");
  const alpha = fn[4] === undefined ? "" : hex(Number(fn[4]) * 255);
  return `#${hex(Number(fn[1]))}${hex(Number(fn[2]))}${hex(Number(fn[3]))}${alpha}`;
}

/** Un worker per linguaggio, caricato solo quando Monaco lo chiede (i servizi TypeScript pesano diversi MB). */
const WORKERS: Record<string, () => Promise<{ default: new () => Worker }>> = {
  json: () => import("monaco-editor/language/json/json.worker?worker"),
  css: () => import("monaco-editor/language/css/css.worker?worker"),
  scss: () => import("monaco-editor/language/css/css.worker?worker"),
  less: () => import("monaco-editor/language/css/css.worker?worker"),
  html: () => import("monaco-editor/language/html/html.worker?worker"),
  handlebars: () => import("monaco-editor/language/html/html.worker?worker"),
  razor: () => import("monaco-editor/language/html/html.worker?worker"),
  typescript: () => import("monaco-editor/language/typescript/ts.worker?worker"),
  javascript: () => import("monaco-editor/language/typescript/ts.worker?worker"),
};
const editorWorker = () => import("monaco-editor/editor/editor.worker?worker");

/** Temi `mc-dark` e `mc-light` dai token `--mc-*` del tema corrente (`data-theme`). */
function defineThemes(monaco: MonacoApi) {
  const css = (name: string) => cssToHex(getComputedStyle(document.documentElement).getPropertyValue(name));
  const colors: Record<string, string | undefined> = {
    "editor.background": css("--mc-bg"),
    "editor.foreground": css("--mc-text"),
    "editorLineNumber.foreground": css("--mc-muted"),
    "editorLineNumber.activeForeground": css("--mc-text"),
    "editorCursor.foreground": css("--mc-accent"),
    "editor.selectionBackground": css("--mc-selection"),
    "editor.lineHighlightBackground": css("--mc-selection-inactive"),
    "editorWidget.background": css("--mc-panel"),
    "editorWidget.border": css("--mc-border"),
    "editorGutter.background": css("--mc-bg"),
    "minimap.background": css("--mc-bg"),
  };
  const defined = Object.fromEntries(Object.entries(colors).filter((e): e is [string, string] => !!e[1]));
  const dark = document.documentElement.dataset.theme === "dark";
  // Si ridefinisce solo il tema attivo: i token dell'altro non sono valorizzati.
  monaco.editor.defineTheme(dark ? "mc-dark" : "mc-light", { base: dark ? "vs-dark" : "vs", inherit: true, rules: [], colors: defined });
  monaco.editor.setTheme(dark ? "mc-dark" : "mc-light");
}

let loading: Promise<MonacoApi> | undefined;

/** Monaco, importato una volta sola. Imposta i worker (Vite: `?worker`) e segue il cambio di `data-theme`. */
export function loadMonaco(): Promise<MonacoApi> {
  return (loading ??= (async () => {
    const monaco = await import("monaco-editor");
    self.MonacoEnvironment = {
      getWorker: async (_id, label) => new (await (WORKERS[label] ?? editorWorker)()).default(),
    };
    defineThemes(monaco);
    new MutationObserver(() => defineThemes(monaco)).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return monaco;
  })().catch((e) => {
    loading = undefined; // si riprova alla prossima apertura
    throw e;
  }));
}
