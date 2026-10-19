// Rispecchia i tipi Rust (src-tauri/src/error.rs, fs/listing.rs, fs/content.rs, fs/drives.rs, fs/hash.rs, archive/mod.rs, ops/model.rs, ops/queue.rs).

export type ErrorCode =
  | "NOT_FOUND"
  | "ACCESS_DENIED"
  | "ALREADY_EXISTS"
  | "DISK_FULL"
  | "NOT_READY"
  | "SAME_FILE"
  | "INTO_ITSELF"
  | "NOT_SAME_DEVICE"
  | "NO_RECYCLE_BIN"
  | "INVALID_NAME"
  | "FILE_IN_USE"
  | "UNSUPPORTED"
  | "CANCELLED"
  | "INVALID_REQUEST"
  | "TOO_LARGE"
  | "CHANGED_ON_DISK"
  | "UNMAPPABLE_CHARS"
  | "CORRUPT_ARCHIVE"
  | "IO";

export interface AppError {
  code: ErrorCode;
  path: string | null;
  detail: string | null;
}

export interface Entry {
  name: string;
  isDir: boolean;
  size: number;
  /** Millisecondi dall'epoca Unix. */
  modifiedMs: number;
  /** Attributi Win32 grezzi. */
  attributes: number;
  isSymlink: boolean;
}

export type DriveKind = "fixed" | "removable" | "network" | "cdrom" | "other";

export interface Drive {
  /** Radice con barra finale, per esempio `C:\`. */
  root: string;
  label: string;
  kind: DriveKind;
  total: number;
  free: number;
  ready: boolean;
}

/** fs/size.rs. `skipped`: sottocartelle illeggibili, non comprese nei totali. */
export interface DirSize {
  bytes: number;
  files: number;
  dirs: number;
  skipped: number;
}

/** fs/content.rs. Date in millisecondi dall'epoca Unix, attributi Win32 grezzi. */
export interface FileInfo {
  size: number;
  isDir: boolean;
  createdMs: number;
  modifiedMs: number;
  accessedMs: number;
  attributes: number;
}

/** fs/scan.rs. `rel`: percorso relativo alla cartella scandita, con `\`. */
export interface ScanEntry {
  rel: string;
  isDir: boolean;
}

export type HashAlgo = "md5" | "sha1" | "sha256";

/** archive/mod.rs. `path` con `\`, senza barra finale. */
export interface ArchiveEntry {
  path: string;
  isDir: boolean;
  size: number;
  modifiedMs: number | null;
}

/** `total`: voci totali, noto per ZIP e 7z; per TAR solo se l'elenco non si è fermato al limite. */
export interface ArchiveListing {
  entries: ArchiveEntry[];
  total: number | null;
  truncated: boolean;
}

export interface DirSizeEvent {
  requestId: number;
  path: string;
  result: DirSize | null;
  error: AppError | null;
}

// Operazioni sui file (ops/model.rs).

export type JobKind =
  | { type: "copy" }
  | { type: "move" }
  | { type: "delete"; permanent: boolean }
  | { type: "mkdir" }
  | { type: "rename" };

export type ConflictPolicy = {
  type: "ask" | "overwrite" | "skip" | "overwriteIfNewer" | "overwriteIfLarger" | "renameNew";
};

/** Copy/Move: `dest` è la cartella di destinazione; Rename: il nuovo percorso completo; Mkdir: `sources[0]`. */
export interface JobSpec {
  kind: JobKind;
  sources: string[];
  dest: string | null;
  /** Maschera TC applicata ai soli file. */
  filter: string | null;
  policy: ConflictPolicy;
}

export interface Meta {
  size: number;
  modifiedMs: number;
}

/** `rename` porta il nuovo percorso completo di destinazione. */
export type Decision =
  | { type: "overwrite" }
  | { type: "skip" }
  | { type: "rename"; path: string }
  | { type: "cancel" };

// Coda dei job (ops/queue.rs).

export type JobId = number;

export type JobState = {
  type: "queued" | "running" | "paused" | "waiting" | "done" | "failed" | "cancelled";
};

export interface JobView {
  id: JobId;
  kind: JobKind;
  state: JobState;
  /** Elementi elaborati (cartelle comprese, anche quelli saltati). */
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  current: string | null;
  /** Media mobile sugli ultimi 2 s. */
  speedBps: number;
  volumes: string[];
  /** Cartella di destinazione di Copy/Move; null per le altre operazioni. */
  dest: string | null;
  /** Un errore per ogni elemento saltato dopo un errore. */
  errors: AppError[];
  /** Prompt in attesa: da qui si recupera se l'evento `job-prompt` è andato perso. */
  prompt: Prompt | null;
}

export type Prompt =
  | {
      type: "conflict";
      promptId: number;
      jobId: JobId;
      src: string;
      dst: string;
      srcMeta: Meta;
      dstMeta: Meta;
      /** Sorgente e destinazione sono lo stesso file: niente "Sovrascrivi". */
      sameFile: boolean;
      /** Percorso completo libero da proporre per "Rinomina". */
      suggestedName: string;
    }
  | {
      type: "error";
      promptId: number;
      jobId: JobId;
      error: AppError;
      canElevate: boolean;
      /** Il Cestino ha rifiutato l'elemento: si può offrire `deletePermanent`. */
      canDeletePermanently: boolean;
    };

/** `retryElevated` si comporta come `retry` finché non c'è l'helper elevato (Task 17). */
export type ErrorAction = {
  type: "retry" | "skip" | "cancel" | "retryElevated" | "deletePermanent";
};

/**
 * `applyToAll`: conflitti → i successivi dello stesso job; errori → i successivi con lo stesso
 * codice (solo `skip`; `deletePermanent` vale per i rifiuti del Cestino successivi; ignorato con `retry`).
 */
export type PromptReply =
  /** `rename`: percorso assoluto nella stessa cartella di `dst`, altrimenti INVALID_REQUEST. */
  | { type: "conflict"; decision: Decision; applyToAll: boolean }
  /** Risolve il conflitto con una politica (non `ask`); con `applyToAll` vale per i successivi. */
  | { type: "conflictPolicy"; policy: ConflictPolicy; applyToAll: boolean }
  | { type: "error"; action: ErrorAction; applyToAll: boolean };
