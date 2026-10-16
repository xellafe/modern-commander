import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Drive, Entry } from "./types";

export const listDir = (path: string) => invoke<Entry[]>("list_dir", { path });

export const nearestExistingAncestor = (path: string) =>
  invoke<string | null>("nearest_existing_ancestor", { path });

export const listDrives = () => invoke<Drive[]>("list_drives");

export const watchDir = (tabId: string, path: string) =>
  invoke<void>("watch_dir", { tabId, path });

export const unwatchDir = (tabId: string) => invoke<void>("unwatch_dir", { tabId });

export const onFsChanged = (cb: (tabId: string) => void): Promise<UnlistenFn> =>
  listen<{ tabId: string }>("fs-changed", (e) => cb(e.payload.tabId));
