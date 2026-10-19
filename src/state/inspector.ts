// Stato dell'ispettore (Ctrl+Q): anteprima, metadati e hash del file sotto il cursore. Il cursore può
// correre (↓ tenuto premuto) quindi `setTarget` aspetta `DEBOUNCE_MS`, e un risultato si applica solo se
// il bersaglio per cui è partito è ancora quello corrente (`token`). Al cambio si annulla tutto ciò che è in corso.
import { create } from "zustand";
import { ansiEncoding, calcHash, cancelRequest, fileInfo, listArchive, nextRequestId, readChunk } from "../ipc/api";
import type { ArchiveListing, DirSize, Drive, ErrorCode, FileInfo, HashAlgo } from "../ipc/types";
import { detectEncoding } from "../lib/encoding";
import { renderMarkdown } from "../lib/markdown";
import { languageFor, loadMonaco } from "../lib/monaco";
import { folderName } from "../lib/paths";
import { looksBinary, previewKind } from "../lib/preview";
import { cancelTabSizes, computeDirSize } from "./sizeRequests";

export const PREVIEW_BYTES = 256 * 1024;
export const DEBOUNCE_MS = 120;
export const ARCHIVE_LIMIT = 500;

/** Proprietario fittizio dei calcoli di dimensione dell'ispettore (`cancelTabSizes`). */
const OWNER = "inspector";

export type Preview =
  | { kind: "loading" }
  | { kind: "image" | "pdf" | "video" | "audio"; path: string }
  /** `language`: id Monaco, null = testo semplice. */
  | { kind: "text"; text: string; language: string | null; truncated: boolean }
  | { kind: "markdown"; html: string; truncated: boolean }
  | { kind: "folder"; size: DirSize | null }
  | { kind: "drive"; drive: Drive }
  | { kind: "archive"; listing: ArchiveListing }
  | { kind: "binary" }
  | { kind: "error"; code: ErrorCode };

export interface InspectorTarget {
  path: string;
  isDir: boolean;
  /** Riga di un'unità nella pseudo-cartella `drives:`. */
  drive?: Drive;
}

interface InspectorState {
  target: InspectorTarget | null;
  info: FileInfo | null;
  preview: Preview | null;
  /** `'pending'` = calcolo in corso. */
  hashes: Partial<Record<HashAlgo, string | "pending">>;
  /** Con debounce; null = ispettore nascosto o niente da mostrare (annulla tutto subito). */
  setTarget: (t: InspectorTarget | null) => void;
  requestHash: (algo: HashAlgo) => void;
}

// Fuori dallo store: non sono stato da mostrare e non devono far rendere nessuno.
let token = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
/** Token del bersaglio mostrato: durante il debounce non coincide con `token` e gli hash non partono. */
let shown = 0;
/** Richieste di hash e archivio in corso, per annullarle al cambio di bersaglio. */
const requests = new Set<number>();

function cancelWork() {
  cancelTabSizes(OWNER);
  for (const id of requests) void cancelRequest(id).catch(() => {});
  requests.clear();
}

const toCode = (e: unknown): ErrorCode =>
  e && typeof e === "object" && "code" in e ? (e as { code: ErrorCode }).code : "IO";

/** Avvia una richiesta annullabile: l'id resta in `requests` finché non finisce. */
async function tracked<T>(run: (id: number) => Promise<T>): Promise<T> {
  const id = nextRequestId();
  requests.add(id);
  try {
    return await run(id);
  } finally {
    requests.delete(id);
  }
}

async function readText(path: string, truncated: boolean): Promise<string | null> {
  const bytes = await readChunk(path, 0, PREVIEW_BYTES);
  if (looksBinary(bytes)) return null;
  const { encoding, bom } = detectEncoding(bytes, await ansiEncoding(), !truncated);
  // `stream` quando il file continua: un carattere spezzato in fondo al campione non diventa U+FFFD.
  return new TextDecoder(encoding).decode(bytes.subarray(bom), { stream: truncated });
}

async function load(t: InspectorTarget, my: number) {
  const live = () => my === token;
  const patch = (p: Partial<InspectorState>) => live() && useInspector.setState(p);
  try {
    if (t.drive) return patch({ preview: { kind: "drive", drive: t.drive } });
    const info = await fileInfo(t.path);
    if (!live()) return;
    useInspector.setState({ info });
    if (t.isDir) {
      patch({ preview: { kind: "folder", size: null } });
      const size = await computeDirSize(OWNER, t.path);
      if (size) patch({ preview: { kind: "folder", size } }); // null = annullato
      return;
    }
    const name = folderName(t.path);
    const kind = previewKind(name);
    if (kind === "image" || kind === "pdf" || kind === "video" || kind === "audio") {
      return patch({ preview: { kind, path: t.path } });
    }
    if (kind === "archive") {
      const listing = await tracked((id) => listArchive(id, t.path, ARCHIVE_LIMIT));
      return patch({ preview: { kind: "archive", listing } });
    }
    const truncated = info.size > PREVIEW_BYTES;
    // Monaco serve per scegliere il linguaggio; se non si carica il testo resta semplice.
    const [read, language] = await Promise.all([
      readText(t.path, truncated),
      kind === "markdown" ? null : loadMonaco().then((m) => languageFor(name, m.languages.getLanguages())).catch(() => null),
    ]);
    if (read === null) return patch({ preview: { kind: "binary" } });
    if (kind === "markdown") return patch({ preview: { kind: "markdown", html: await renderMarkdown(read), truncated } });
    patch({ preview: { kind: "text", text: read, language, truncated } });
  } catch (e) {
    patch({ preview: { kind: "error", code: toCode(e) } });
  }
}

export const useInspector = create<InspectorState>((set, get) => ({
  target: null,
  info: null,
  preview: null,
  hashes: {},
  setTarget: (t) => {
    clearTimeout(timer);
    const my = ++token;
    cancelWork();
    if (!t) return set({ target: null, info: null, preview: null, hashes: {} });
    timer = setTimeout(() => {
      shown = my;
      set({ target: t, info: null, preview: { kind: "loading" }, hashes: {} });
      void load(t, my);
    }, DEBOUNCE_MS);
  },
  requestHash: (algo) => {
    const { target, hashes } = get();
    if (shown !== token || !target || target.isDir || target.drive || hashes[algo] === "pending") return;
    const my = token;
    set({ hashes: { ...hashes, [algo]: "pending" } });
    tracked((id) => calcHash(id, target.path, algo)).then(
      (hash) => my === token && set((s) => ({ hashes: { ...s.hashes, [algo]: hash } })),
      () => {
        if (my !== token) return;
        set((s) => {
          const { [algo]: _, ...rest } = s.hashes;
          return { hashes: rest };
        });
      },
    );
  },
}));
