// Rispecchia i tipi Rust (src-tauri/src/error.rs, fs/listing.rs, fs/drives.rs, ops/model.rs, ops/queue.rs).

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
  | "CANCELLED"
  | "INVALID_REQUEST"
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
  /** Un errore per ogni elemento saltato dopo un errore. */
  errors: AppError[];
}

export type Prompt =
  | { type: "conflict"; promptId: number; jobId: JobId; src: string; dst: string; srcMeta: Meta; dstMeta: Meta }
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
  | { type: "conflict"; decision: Decision; applyToAll: boolean }
  | { type: "error"; action: ErrorAction; applyToAll: boolean };
