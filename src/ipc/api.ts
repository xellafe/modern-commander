import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ArchiveListing, DirSizeEvent, Drive, Entry, FileInfo, HashAlgo, JobId, JobSpec, JobView, Prompt, PromptReply, ScanEntry, ShellKind, TermExitEvent } from "./types";

export const listDir = (path: string) => invoke<Entry[]>("list_dir", { path });

export const nearestExistingAncestor = (path: string) =>
  invoke<string | null>("nearest_existing_ancestor", { path });

export const listDrives = () => invoke<Drive[]>("list_drives");

/** Forma lunga dei percorsi (i nomi 8.3 del drop da Explorer); quelli non risolvibili restano uguali. */
export const normalizePaths = (paths: string[]) => invoke<string[]>("normalize_paths", { paths });

export const watchDir = (tabId: string, path: string) =>
  invoke<void>("watch_dir", { tabId, path });

export const unwatchDir = (tabId: string) => invoke<void>("unwatch_dir", { tabId });

export const onFsChanged = (cb: (tabId: string) => void): Promise<UnlistenFn> =>
  listen<{ tabId: string }>("fs-changed", (e) => cb(e.payload.tabId));

/** Risponde subito: il risultato arriva con l'evento `dir-size` (vedi `onDirSize`). */
export const calcDirSize = (requestId: number, path: string) =>
  invoke<void>("calc_dir_size", { requestId, path });

// Parte dall'ora: dopo un ricaricamento della pagina il registro del backend e gli eventi in volo sopravvivono, e un id
// ripartito da 1 risolverebbe (o sovrascriverebbe) la richiesta sbagliata.
let lastRequestId = Date.now();

/** Id per le richieste annullabili (dimensione, hash, scansione, archivi): unico per tutta l'app. */
export const nextRequestId = () => ++lastRequestId;

/** Alza il flag di annullamento della richiesta; un id già finito o sconosciuto non ha effetto. */
export const cancelRequest = (requestId: number) => invoke<void>("cancel_request", { requestId });

/**
 * Scandisce l'albero sotto `root` in ampiezza: le voci arrivano a lotti su `onBatch` mentre la richiesta è in corso;
 * si risolve a scansione finita con `true` se si è fermata al limite di voci. `cancelRequest(requestId)` la annulla
 * (errore `CANCELLED`).
 */
export const scanTree = (
  requestId: number,
  root: string,
  limits: { maxDepth: number; maxEntries: number; includeHidden: boolean },
  onBatch: (batch: ScanEntry[]) => void,
) => {
  const channel = new Channel<ScanEntry[]>();
  channel.onmessage = onBatch;
  return invoke<boolean>("scan_tree", { requestId, root, ...limits, onBatch: channel });
};

/**
 * Byte dopo il primo LF a `from` o oltre (LF su confini di unità: `unit` 1 = UTF-8/ANSI, 2 = UTF-16); senza LF, la
 * dimensione del file. `onProgress` riceve l'offset fino a cui si è già cercato. `cancelRequest(requestId)` annulla.
 */
export const nextLineStart = (
  requestId: number,
  path: string,
  from: number,
  unit: 1 | 2,
  bigEndian: boolean,
  onProgress: (offset: number) => void,
) => {
  const channel = new Channel<number>();
  channel.onmessage = onProgress;
  return invoke<number>("next_line_start", { requestId, path, from, unit, bigEndian, onProgress: channel });
};

/**
 * Inizio della riga che contiene `at` (0 senza LF prima). `onProgress` riceve l'offset più basso già esaminato.
 * `cancelRequest(requestId)` annulla.
 */
export const lineStart = (
  requestId: number,
  path: string,
  at: number,
  unit: 1 | 2,
  bigEndian: boolean,
  onProgress: (offset: number) => void,
) => {
  const channel = new Channel<number>();
  channel.onmessage = onProgress;
  return invoke<number>("line_start", { requestId, path, at, unit, bigEndian, onProgress: channel });
};

/** Hash del file in esadecimale minuscolo; `cancelRequest(requestId)` lo annulla (errore `CANCELLED`). */
export const calcHash = (requestId: number, path: string, algo: HashAlgo) =>
  invoke<string>("calc_hash", { requestId, path, algo });

/**
 * Prime `limit` voci dell'archivio (ZIP, 7z, TAR, TAR.GZ). `UNSUPPORTED` per estensione non riconosciuta o 7z con
 * intestazioni cifrate (detail `encrypted`), `CORRUPT_ARCHIVE` se illeggibile; `cancelRequest(requestId)` lo annulla.
 */
export const listArchive = (requestId: number, path: string, limit: number) =>
  invoke<ArchiveListing>("list_archive", { requestId, path, limit });

export const onDirSize = (cb: (e: DirSizeEvent) => void): Promise<UnlistenFn> =>
  listen<DirSizeEvent>("dir-size", (e) => cb(e.payload));

export const fileInfo = (path: string) => invoke<FileInfo>("file_info", { path });

/** Fino a `len` byte (al massimo 4 MiB) da `offset`; oltre la fine del file l'array è più corto o vuoto. */
export const readChunk = async (path: string, offset: number, len: number) =>
  new Uint8Array(await invoke<ArrayBuffer>("read_chunk", { path, offset, len }));

/** Tutto il file; TOO_LARGE se supera `max` byte. */
export const readFile = async (path: string, max: number) =>
  new Uint8Array(await invoke<ArrayBuffer>("read_file", { path, max }));

/**
 * Salva il testo dell'editor senza mai scrivere sul posto (`<file>.mcpart` poi sostituzione). Con una data
 * attesa `CHANGED_ON_DISK` se il file è cambiato; `null` sovrascrive. `UNMAPPABLE_CHARS` se la codifica non basta.
 */
export const writeTextFile = (path: string, text: string, encoding: string, bom: boolean, expectedModifiedMs: number | null) =>
  invoke<FileInfo>("write_text_file", { path, text, encoding, bom, expectedModifiedMs });

let ansi: Promise<string> | undefined;

/** Etichetta `TextDecoder` della codifica ANSI di sistema; si chiede una volta sola (si riprova se fallisce). */
export const ansiEncoding = () =>
  (ansi ??= invoke<string>("ansi_encoding").catch((e) => {
    ansi = undefined;
    throw e;
  }));

/** Menu contestuale della shell; `x`/`y` in pixel fisici, client della finestra. Stessa cartella padre per tutti. */
export const showContextMenu = (paths: string[], x: number, y: number) =>
  invoke<void>("show_context_menu", { paths, x, y });

export const showProperties = (paths: string[]) => invoke<void>("show_properties", { paths });

/** Drag modale verso Explorer: si risolve a drop concluso o annullato (no-op a pulsante già rilasciato). */
export const startDrag = (paths: string[]) => invoke<void>("start_drag", { paths });

/** Apre con il programma associato (verbo "open" della shell). */
export const openPath = (path: string) => invoke<void>("open_path", { path });

/** Apre `path` con l'eseguibile `exe`, senza attendere che termini. */
export const openWith = (exe: string, path: string) => invoke<void>("open_with", { exe, path });

/**
 * Miniatura della shell (o icona del tipo) con il lato maggiore ≤ `size` (16–1024), come RGBA: il backend manda
 * `[larghezza u32 LE][altezza u32 LE][pixel]`.
 */
export const thumbnail = async (path: string, size: number) => {
  const buf = await invoke<ArrayBuffer>("get_thumbnail", { path, size });
  const head = new DataView(buf);
  return { width: head.getUint32(0, true), height: head.getUint32(4, true), data: new Uint8ClampedArray(buf, 8) };
};

/** Shell avviabili (eseguibile nel PATH), nell'ordine pwsh, powershell, cmd, wsl. */
export const termShells = () => invoke<ShellKind[]>("term_shells");

/**
 * Apre un terminale ConPTY e ne restituisce l'id; `shell` nullo sceglie la migliore disponibile, `cwd` vuota o non
 * valida diventa la cartella utente. L'output arriva a blocchi grezzi su `onOutput` (un carattere UTF-8 può essere
 * spezzato tra due blocchi: lo ricompone xterm.js). Il primo blocco contiene una richiesta di posizione del cursore
 * a cui il terminale deve rispondere (xterm.js lo fa da sé, via `onData`) o la shell non parte.
 *
 * Contratto: `onOutput` può scattare PRIMA che la promessa si risolva, cioè quando l'id ancora non c'è. Chi chiama
 * deve quindi collegare `onData` di xterm prima di chiamare `termSpawn`, tenendo in coda le scritture finché non
 * ha l'id; con `const id = await termSpawn(...); term.onData((d) => termWrite(id, d))` la risposta al cursore si
 * perde e la shell resta appesa. Lo stesso vale per `onTermExit`, che può arrivare prima della promessa se il
 * processo muore subito, e i cui ultimi blocchi di output possono raggiungere `onOutput` anche dopo l'evento:
 * alla ricezione di `term-exit` il terminale non va distrutto.
 */
export const termSpawn = (shell: ShellKind | null, cwd: string, cols: number, rows: number, onOutput: (bytes: Uint8Array) => void) => {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buf) => onOutput(new Uint8Array(buf));
  return invoke<number>("term_spawn", { shell, cwd, cols, rows, onOutput: channel });
};

export const termWrite = (id: number, data: string) => invoke<void>("term_write", { id, data });

export const termResize = (id: number, cols: number, rows: number) => invoke<void>("term_resize", { id, cols, rows });

/** Ferma la shell; la chiusura arriva poi come `term-exit`. */
export const termKill = (id: number) => invoke<void>("term_kill", { id });

export const onTermExit = (cb: (e: TermExitEvent) => void): Promise<UnlistenFn> =>
  listen<TermExitEvent>("term-exit", (e) => cb(e.payload));

export const readConfig = (name: string) => invoke<unknown | null>("read_config", { name });

export const writeConfig = (name: string, value: unknown) =>
  invoke<void>("write_config", { name, value });

/** Restituisce subito l'id: l'espansione e il lavoro avvengono nel worker del job. */
export const enqueueJob = (spec: JobSpec) => invoke<JobId>("enqueue_job", { spec });

export const pauseJob = (id: JobId) => invoke<void>("pause_job", { id });

export const resumeJob = (id: JobId) => invoke<void>("resume_job", { id });

export const cancelJob = (id: JobId) => invoke<void>("cancel_job", { id });

/** Sposta il job alla posizione `index` della coda. */
export const moveJob = (id: JobId, index: number) => invoke<void>("move_job", { id, index });

export const replyPrompt = (promptId: number, reply: PromptReply) =>
  invoke<void>("reply_prompt", { promptId, reply });

/** I job nell'ordine della coda, compresi quelli finiti. */
export const listJobs = () => invoke<JobView[]>("list_jobs");

/** Toglie dall'elenco i job Done, Failed e Cancelled. */
export const clearFinished = () => invoke<void>("clear_finished");

/** Al massimo un evento ogni 100 ms per job, più uno a ogni cambio di stato. */
export const onJobUpdated = (cb: (job: JobView) => void): Promise<UnlistenFn> =>
  listen<JobView>("job-updated", (e) => cb(e.payload));

export const onJobPrompt = (cb: (prompt: Prompt) => void): Promise<UnlistenFn> =>
  listen<Prompt>("job-prompt", (e) => cb(e.payload));
