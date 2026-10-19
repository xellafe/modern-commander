import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ArchiveListing, DirSizeEvent, Drive, Entry, FileInfo, HashAlgo, JobId, JobSpec, JobView, Prompt, PromptReply, ScanEntry } from "./types";

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

let lastRequestId = 0;

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
