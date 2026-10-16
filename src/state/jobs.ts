// Stato della coda delle operazioni (Task 15): job, ordine e prompt in attesa.
import { create } from "zustand";
import { listJobs, onJobPrompt, onJobUpdated, replyPrompt } from "../ipc/api";
import type { JobId, JobView, Prompt, PromptReply } from "../ipc/types";
import { joinPath } from "../lib/paths";
import { parentPath } from "./tabView";
import { useUi } from "./ui";

export const isFinished = (j: JobView) => ["done", "failed", "cancelled"].includes(j.state.type);

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
  const jobs = { ...s.jobs, [job.id]: job };
  useJobs.setState({ jobs, order: isNew ? [...s.order, job.id] : s.order, prompts: pendingPrompts(jobs, answered) });
  // Ogni nuovo job apre il pannello sulla scheda Operazioni.
  if (isNew && job.state.type === "queued") useUi.setState({ bottomPanelVisible: true, bottomPanelTab: "operations" });
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

/** Attende la fine del job (anche se è già finito). */
export function waitForJob(id: JobId): Promise<JobView> {
  return new Promise((resolve) => {
    const check = (s: JobsState) => {
      const j = s.jobs[id];
      if (j && isFinished(j)) {
        off();
        resolve(j);
        return true;
      }
      return false;
    };
    const off = useJobs.subscribe(check);
    check(useJobs.getState());
  });
}

/** Dato per la barra di stato: il primo job avviato e non finito, nell'ordine della coda. */
export const useStatusJob = () =>
  useJobs((s) => {
    const id = s.order.find((i) => ["running", "waiting", "paused"].includes(s.jobs[i]?.state.type ?? ""));
    return id === undefined ? null : s.jobs[id];
  });
