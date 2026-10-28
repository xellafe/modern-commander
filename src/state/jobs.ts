// Stato della coda delle operazioni (Task 15): job, ordine e prompt in attesa.
import { create } from "zustand";
import { cancelJob, listJobs, onJobPrompt, onJobUpdated, replyPrompt } from "../ipc/api";
import type { JobId, JobKind, JobView, Prompt, PromptReply } from "../ipc/types";
import { isArchive, joinPath, parseRemote } from "../lib/paths";
import { useLayout } from "./layout";
import { parentPath } from "./tabView";
import { useTabViews } from "./tabViews";
import { useUi } from "./ui";

export const isFinished = (j: JobView) => ["done", "failed", "cancelled"].includes(j.state.type);

export const kindKey = (k: JobKind) => (k.type === "delete" && k.permanent ? "ops.kind.deletePermanent" : `ops.kind.${k.type}`);

/** Avanzamento da 0 a 1: per byte se ce ne sono, altrimenti per elementi. */
export function progressOf(j: JobView): number {
  if (j.state.type === "done") return 1;
  if (j.bytesTotal > 0) return j.bytesDone / j.bytesTotal;
  return j.filesTotal > 0 ? j.filesDone / j.filesTotal : 0;
}

/** Prompt in attesa, il più vecchio per primo (gli id crescono nel backend). */
export const pendingPrompts = (jobs: Record<JobId, JobView>, answered: Set<number>): Prompt[] =>
  Object.values(jobs)
    .flatMap((j) => (j.prompt && !answered.has(j.prompt.promptId) ? [j.prompt] : []))
    .sort((a, b) => a.promptId - b.promptId);

export type ConflictChoice = "overwrite" | "skip" | "rename" | "newer" | "larger" | "cancel";

/** Pulsanti del conflitto: sullo stesso file sovrascrivere (anche per data o dimensione) darebbe solo `SAME_FILE`. */
export const conflictChoices = (sameFile: boolean): ConflictChoice[] =>
  sameFile ? ["skip", "rename", "cancel"] : ["overwrite", "skip", "rename", "newer", "larger", "cancel"];

export function conflictReply(choice: ConflictChoice, applyToAll: boolean, renameTo = ""): PromptReply {
  switch (choice) {
    case "newer": return { type: "conflictPolicy", policy: { type: "overwriteIfNewer" }, applyToAll };
    case "larger": return { type: "conflictPolicy", policy: { type: "overwriteIfLarger" }, applyToAll };
    case "rename": return { type: "conflict", decision: { type: "rename", path: renameTo }, applyToAll };
    case "cancel": return { type: "conflict", decision: { type: "cancel" }, applyToAll: false };
    default: return { type: "conflict", decision: { type: choice }, applyToAll };
  }
}

/**
 * Connessioni remote (`sftp://w`) toccate dal job: le schede lì non hanno watcher. Un archivio remoto dà due volumi,
 * `arc://` + la chiave del contenitore (`arc://sftp://w/rel/a.zip`) e la sua connessione: contano una volta.
 */
export const remoteVolumesOf = (job: JobView): string[] => [...new Set(job.volumes.flatMap((v) => {
  const r = parseRemote(v.replace(/^arc:\/\//i, ""));
  return r ? [`${r.scheme}://${r.conn}`] : [];
}))];

/**
 * Rilegge le schede sulle connessioni toccate dal job, al posto del watcher che il remoto non ha, e tutte le schede
 * d'archivio aperte. ponytail: anche quelle che il job non ha toccato; la cache dell'indice rende la rilettura economica.
 */
function reloadRemoteTabs(job: JobView) {
  const vols = remoteVolumesOf(job);
  for (const tab of Object.values(useLayout.getState().tabs)) {
    const r = parseRemote(tab.path);
    if (isArchive(tab.path) || (r && vols.includes(`${r.scheme}://${r.conn}`))) void useTabViews.getState().reload(tab.id);
  }
}

/** Nuovo percorso per "Rinomina": la cartella resta quella del nome proposto. */
export const renamePath = (suggested: string, name: string) => joinPath(parentPath(suggested) ?? suggested, name);

interface JobsState {
  jobs: Record<JobId, JobView>;
  /** Ordine della coda (quello del backend). */
  order: JobId[];
  prompts: Prompt[];
  /** Risponde e, se il backend accetta, toglie il prompt; altrimenti rilancia l'errore. */
  answer: (prompt: Prompt, reply: PromptReply) => Promise<void>;
  /** Rilegge elenco e ordine (move_job e clear_finished non emettono eventi). */
  refresh: () => Promise<void>;
}

// Prompt già risposti: un evento arrivato in ritardo non li deve riaprire.
const answered = new Set<number>();

export const useJobs = create<JobsState>((set, get) => ({
  jobs: {},
  order: [],
  prompts: [],
  answer: async (prompt, reply) => {
    await replyPrompt(prompt.promptId, reply);
    answered.add(prompt.promptId);
    set((s) => ({ prompts: pendingPrompts(s.jobs, answered) }));
  },
  refresh: async () => {
    touched.clear();
    const list = await listJobs();
    // Un job aggiornato da un evento mentre la lettura era in corso tiene il valore dell'evento:
    // o è il più recente, o ne seguirà un altro.
    const local = get().jobs;
    const jobs = Object.fromEntries(list.map((j) => [j.id, touched.has(j.id) ? (local[j.id] ?? j) : j]));
    const order = list.map((j) => j.id);
    for (const id of touched) {
      if (!(id in jobs) && local[id]) {
        jobs[id] = local[id]; // accodato dopo la lettura
        order.push(id);
      }
    }
    set({ jobs, order, prompts: pendingPrompts(jobs, answered) });
  },
}));

const touched = new Set<JobId>();

function upsert(job: JobView) {
  touched.add(job.id);
  const s = useJobs.getState();
  const isNew = !(job.id in s.jobs);
  const ended = isFinished(job) && (isNew || !isFinished(s.jobs[job.id]));
  const jobs = { ...s.jobs, [job.id]: job };
  useJobs.setState({ jobs, order: isNew ? [...s.order, job.id] : s.order, prompts: pendingPrompts(jobs, answered) });
  // Ogni nuovo job apre il pannello sulla scheda Operazioni.
  if (isNew && job.state.type === "queued") useUi.setState({ bottomPanelVisible: true, bottomPanelTab: "operations" });
  if (ended) reloadRemoteTabs(job);
}

/** Eventi della coda e stato iniziale (anche dopo il ricaricamento della webview: prompt compresi). */
export async function initJobs() {
  await onJobUpdated(upsert);
  // Di norma il prompt arriva già col job-updated di stato `waiting`: questo copre il resto.
  await onJobPrompt((p) => {
    const j = useJobs.getState().jobs[p.jobId];
    if (j) upsert({ ...j, prompt: p });
  });
  await useJobs.getState().refresh();
}

/**
 * Attende la fine del job (anche se è già finito). Un job tolto dall'elenco prima del suo evento
 * finale (clear_finished) conta come annullato: non si resta in attesa per sempre.
 */
export function waitForJob(id: JobId): Promise<JobView> {
  return new Promise((resolve) => {
    let seen: JobView | undefined;
    const check = (s: JobsState) => {
      const j = s.jobs[id];
      const end = j ? (isFinished(j) ? j : undefined) : seen && { ...seen, state: { type: "cancelled" as const } };
      seen = j ?? seen;
      if (!end) return;
      off();
      resolve(end);
    };
    const off = useJobs.subscribe(check);
    check(useJobs.getState());
  });
}

/** Job non ancora finiti (in coda, in corso, in pausa, in attesa di risposta). */
export const unfinishedJobs = (jobs: Record<JobId, JobView>): JobView[] => Object.values(jobs).filter((j) => !isFinished(j));

/**
 * Annulla tutti i job non finiti e attende il loro stato finale, così una copia in corso toglie il
 * proprio `.mcpart`; al massimo `timeoutMs`, poi si prosegue comunque (un job bloccato non trattiene l'uscita).
 */
export async function cancelAllAndWait(timeoutMs: number): Promise<void> {
  const ids = unfinishedJobs(useJobs.getState().jobs).map((j) => j.id);
  await Promise.all(ids.map((id) => cancelJob(id).catch(() => {})));
  let timer: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    Promise.all(ids.map(waitForJob)),
    new Promise((r) => (timer = setTimeout(r, timeoutMs))),
  ]);
  clearTimeout(timer);
}

/** Job avviati e non finiti (in corso, in attesa di risposta, in pausa), nell'ordine della coda: la barra di stato mostra il primo. */
export const activeJobs = (order: JobId[], jobs: Record<JobId, JobView>): JobView[] =>
  order.flatMap((i) => (["running", "waiting", "paused"].includes(jobs[i]?.state.type ?? "") ? [jobs[i]] : []));

/** Il job aggiunto dopo `seen` (gli id crescono), se c'è: un riordino o una rimozione non ne creano uno. */
export function newestJob(order: JobId[], seen: JobId): JobId | null {
  const max = Math.max(0, ...order);
  return max > seen ? max : null;
}
