// Rispecchia i tipi Rust (src-tauri/src/error.rs, fs/listing.rs, fs/drives.rs).

export type ErrorCode =
  | "NOT_FOUND"
  | "ACCESS_DENIED"
  | "ALREADY_EXISTS"
  | "DISK_FULL"
  | "NOT_READY"
  | "SAME_FILE"
  | "INTO_ITSELF"
  | "NOT_SAME_DEVICE"
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
