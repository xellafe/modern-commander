// Stato degli editor F4 per scheda. Il documento (percorso, codifica, data letta, modifiche) sta nello store;
// i modelli di Monaco no: sono oggetti con vita propria, e cambiando scheda il corpo del gruppo si smonta ma
// il modello resta, così le modifiche non salvate (e la cronologia di undo) sopravvivono.
import type { editor } from "monaco-editor";
import { create } from "zustand";
import { ansiEncoding, fileInfo, readFile, writeTextFile } from "../ipc/api";
import type { AppError } from "../ipc/types";
import { detectEncoding } from "../lib/encoding";
import { folderName } from "../lib/paths";
import { languageFor, loadMonaco } from "../lib/monaco";
import { looksBinary } from "../lib/preview";
import { useTabViews } from "./tabViews";
import { useUi } from "./ui";

/** Oltre questa dimensione F4 non apre il file (resta F3). */
export const EDITOR_MAX_BYTES = 32 * 1024 * 1024;

export interface EditorDoc {
  path: string;
  /** Etichetta WHATWG con cui il file si rilegge e si riscrive. */
  encoding: string;
  bom: boolean;
  /** Data di modifica vista all'apertura o all'ultimo salvataggio: se su disco è diversa, il file è cambiato. */
  modifiedMs: number;
  dirty: boolean;
  /** File binario: decodificarlo e riscriverlo lo corromperebbe. */
  readOnly: boolean;
}

/**
 * Testo (senza BOM) e codifica di un file letto per intero. `readOnly` se sembra binario o se la decodifica
 * perderebbe dati (byte non validi o senza corrispondenza): il testo si mostra comunque, ma riscriverlo
 * sostituirebbe quei byte con U+FFFD.
 */
export function decodeForEditor(bytes: Uint8Array, ansi: string): { text: string; encoding: string; bom: boolean; readOnly: boolean } {
  const { encoding, bom } = detectEncoding(bytes, ansi, true);
  const body = bytes.subarray(bom);
  let text: string;
  let lossy = false;
  try {
    // `ignoreBOM`: il BOM è già saltato, un U+FEFF che resta fa parte del testo.
    text = new TextDecoder(encoding, { fatal: true, ignoreBOM: true }).decode(body);
  } catch {
    lossy = true;
    text = new TextDecoder(encoding, { ignoreBOM: true }).decode(body);
  }
  return { text, encoding, bom: bom > 0, readOnly: lossy || looksBinary(bytes) };
}

// Fuori dallo store: modelli, versione dell'ultimo salvataggio, posizione di scorrimento e aperture in corso.
const models = new Map<string, editor.ITextModel>();
const savedVersion = new Map<string, number>();
const viewStates = new Map<string, editor.ICodeEditorViewState | null>();
const loads = new Map<string, { promise: Promise<void> }>();
/** Coda dei salvataggi per file (percorso minuscolo) e salvataggio semplice in coda per scheda. */
const saveQueues = new Map<string, Promise<void>>();
const waitingSaves = new Map<string, Promise<void>>();

export const editorModel = (tabId: string) => models.get(tabId);
export const saveViewState = (tabId: string, state: editor.ICodeEditorViewState | null) => void viewStates.set(tabId, state);
export const savedViewState = (tabId: string) => viewStates.get(tabId) ?? null;

/** Schede editor con modifiche non salvate (la chiusura della finestra le conta). */
export const dirtyEditorCount = () => Object.values(useEditors.getState().docs).filter((d) => d.dirty).length;

export interface SaveOptions {
  /** Sovrascrive anche se il file è cambiato su disco. */
  force?: boolean;
  /** Salva in un'altra codifica. */
  encoding?: string;
}

/**
 * Opzioni con cui il dialogo di errore ripete il salvataggio: quelle del tentativo fallito più la correzione
 * scelta. Senza la somma, un file cambiato su disco E con caratteri non rappresentabili non si salverebbe mai.
 */
export const retryOptions = (previous: SaveOptions, code: "CHANGED_ON_DISK" | "UNMAPPABLE_CHARS"): SaveOptions =>
  code === "CHANGED_ON_DISK" ? { ...previous, force: true } : { ...previous, encoding: "utf-8" };

interface EditorsStore {
  docs: Record<string, EditorDoc>;
  /** Legge il file e crea il modello; già aperto (ritorno alla scheda) non fa niente. Rifiuta con l'errore del backend. */
  open: (tabId: string, path: string) => Promise<void>;
  setDirty: (tabId: string, dirty: boolean) => void;
  /**
   * I salvataggi dello stesso file (anche da due schede) vanno uno alla volta; più richieste semplici
   * della stessa scheda ancora in coda si fondono in una.
   */
  save: (tabId: string, opts?: SaveOptions) => Promise<void>;
  dispose: (tabId: string) => void;
}

const patchDoc = (tabId: string, patch: Partial<EditorDoc>) =>
  useEditors.setState((s) => (s.docs[tabId] ? { docs: { ...s.docs, [tabId]: { ...s.docs[tabId], ...patch } } } : s));

/** `dirty` dal modello: un undo fino al punto salvato riporta la stessa versione alternativa, quindi pulito. */
function refreshDirty(tabId: string) {
  const model = models.get(tabId);
  if (model) useEditors.getState().setDirty(tabId, model.getAlternativeVersionId() !== savedVersion.get(tabId));
}

async function load(tabId: string, path: string, current: () => boolean) {
  // La data si legge prima del contenuto: se il file cambia nel mezzo, il salvataggio lo vedrà come cambiato.
  const [info, ansi, monaco] = await Promise.all([fileInfo(path), ansiEncoding(), loadMonaco()]);
  const decoded = decodeForEditor(await readFile(path, EDITOR_MAX_BYTES), ansi);
  if (!current()) return; // scheda chiusa durante la lettura: niente modello né documento
  const model = monaco.editor.createModel(decoded.text, languageFor(folderName(path), monaco.languages.getLanguages()) ?? "plaintext");
  models.set(tabId, model);
  savedVersion.set(tabId, model.getAlternativeVersionId());
  model.onDidChangeContent(() => refreshDirty(tabId));
  const { text: _text, ...rest } = decoded;
  useEditors.setState((s) => ({ docs: { ...s.docs, [tabId]: { path, ...rest, modifiedMs: info.modifiedMs, dirty: false } } }));
}

/** Un salvataggio: errori in dialogo o avviso, mai rifiutati. */
async function writeDoc(tabId: string, opts: SaveOptions) {
  const doc = useEditors.getState().docs[tabId];
  const model = models.get(tabId);
  if (!doc || !model || doc.readOnly) return;
  const version = model.getAlternativeVersionId();
  const encoding = opts.encoding ?? doc.encoding;
  try {
    const info = await writeTextFile(doc.path, model.getValue(), encoding, doc.bom, opts.force ? null : doc.modifiedMs);
    if (models.get(tabId) !== model) return; // scheda chiusa durante la scrittura
    savedVersion.set(tabId, version);
    patchDoc(tabId, { encoding, modifiedMs: info.modifiedMs });
    refreshDirty(tabId); // se si è scritto durante il salvataggio resta modificato
  } catch (e) {
    const code = (e as Partial<AppError> | null)?.code;
    if (code === "CHANGED_ON_DISK" || code === "UNMAPPABLE_CHARS") useUi.getState().openDialog({ kind: "saveError", tabId, code, opts });
    else useTabViews.getState().showNotice(tabId, e);
  }
}

export const useEditors = create<EditorsStore>((set, get) => ({
  docs: {},

  open: (tabId, path) => {
    if (get().docs[tabId]) return Promise.resolve();
    const running = loads.get(tabId);
    if (running) return running.promise;
    const entry = {} as { promise: Promise<void> };
    entry.promise = load(tabId, path, () => loads.get(tabId) === entry).finally(() => {
      if (loads.get(tabId) === entry) loads.delete(tabId);
    });
    loads.set(tabId, entry);
    return entry.promise;
  },

  setDirty: (tabId, dirty) => {
    if (get().docs[tabId]?.dirty !== dirty) patchDoc(tabId, { dirty });
  },

  save: (tabId, opts = {}) => {
    const path = get().docs[tabId]?.path;
    if (path === undefined) return Promise.resolve();
    const plain = !opts.force && !opts.encoding;
    const waiting = plain ? waitingSaves.get(tabId) : undefined;
    if (waiting) return waiting; // parte più tardi e legge il testo di allora: questa richiesta è già coperta
    const key = path.toLowerCase();
    // `writeDoc` non rifiuta mai, quindi la coda non si rompe.
    const turn = (saveQueues.get(key) ?? Promise.resolve()).then(() => {
      if (plain) waitingSaves.delete(tabId);
      return writeDoc(tabId, opts);
    });
    saveQueues.set(key, turn);
    if (plain) waitingSaves.set(tabId, turn);
    void turn.then(() => saveQueues.get(key) === turn && saveQueues.delete(key));
    return turn;
  },

  dispose: (tabId) => {
    models.get(tabId)?.dispose();
    models.delete(tabId);
    savedVersion.delete(tabId);
    viewStates.delete(tabId);
    loads.delete(tabId);
    set((s) => {
      const { [tabId]: _removed, ...docs } = s.docs;
      return { docs };
    });
  },
}));
