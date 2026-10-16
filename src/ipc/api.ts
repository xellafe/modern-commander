import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Drive, Entry, JobId, JobSpec, JobView, Prompt, PromptReply } from "./types";

export const listDir = (path: string) => invoke<Entry[]>("list_dir", { path });

export const nearestExistingAncestor = (path: string) =>
  invoke<string | null>("nearest_existing_ancestor", { path });

export const listDrives = () => invoke<Drive[]>("list_drives");

export const watchDir = (tabId: string, path: string) =>
  invoke<void>("watch_dir", { tabId, path });

export const unwatchDir = (tabId: string) => invoke<void>("unwatch_dir", { tabId });

export const onFsChanged = (cb: (tabId: string) => void): Promise<UnlistenFn> =>
  listen<{ tabId: string }>("fs-changed", (e) => cb(e.payload.tabId));

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
