import { expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { JobView, Prompt } from "../ipc/types";
import { cancelAllAndWait, conflictChoices, conflictReply, initJobs, newestJob, pendingPrompts, remoteVolumesOf, renamePath, unfinishedJobs, useJobs, waitForJob } from "./jobs";
import { useLayout } from "./layout";
import { useTabViews } from "./tabViews";

test("newestJob: only a job added after the last one seen, not a reorder or a removal", () => {
  expect(newestJob([], 0)).toBeNull();
  expect(newestJob([1, 2], 0)).toBe(2);
  expect(newestJob([1, 2], 2)).toBeNull();
  expect(newestJob([2, 1], 2)).toBeNull(); // riordino col trascinamento: l'ultima riga cambia, nessun job nuovo
  expect(newestJob([3, 1, 2], 2)).toBe(3); // il nuovo può non essere l'ultimo dopo un riordino
  expect(newestJob([1], 2)).toBeNull(); // "Rimuovi finite" ha tolto il più recente
});

test("conflictChoices on the same file offers only skip, rename and cancel", () => {
  expect(conflictChoices(true)).toEqual(["skip", "rename", "cancel"]);
  expect(conflictChoices(false)).toEqual(["overwrite", "skip", "rename", "newer", "larger", "cancel"]);
});

const job = (id: number, prompt: Prompt | null): JobView => ({
  id, kind: { type: "copy" }, state: { type: prompt ? "waiting" : "running" },
  filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, current: null, speedBps: 0,
  volumes: [], dest: null, errors: [], prompt,
});
const err = (promptId: number, jobId: number): Prompt => ({
  type: "error", promptId, jobId, error: { code: "IO", path: null, detail: null },
  canElevate: false, canDeletePermanently: false,
});

test("pendingPrompts lists the waiting prompts oldest first, without the answered ones", () => {
  const jobs = { 1: job(1, err(7, 1)), 2: job(2, null), 3: job(3, err(4, 3)), 4: job(4, err(9, 4)) };
  expect(pendingPrompts(jobs, new Set()).map((p) => p.promptId)).toEqual([4, 7, 9]);
  expect(pendingPrompts(jobs, new Set([4])).map((p) => p.promptId)).toEqual([7, 9]);
});

test("conflictReply maps every choice to the backend shape", () => {
  expect(conflictReply("overwrite", true)).toEqual({ type: "conflict", decision: { type: "overwrite" }, applyToAll: true });
  expect(conflictReply("skip", false)).toEqual({ type: "conflict", decision: { type: "skip" }, applyToAll: false });
  expect(conflictReply("rename", false, "C:\\d\\f (2).txt")).toEqual({
    type: "conflict", decision: { type: "rename", path: "C:\\d\\f (2).txt" }, applyToAll: false,
  });
  expect(conflictReply("newer", true)).toEqual({ type: "conflictPolicy", policy: { type: "overwriteIfNewer" }, applyToAll: true });
  expect(conflictReply("larger", false)).toEqual({ type: "conflictPolicy", policy: { type: "overwriteIfLarger" }, applyToAll: false });
  // Annulla chiude il job: "per tutti" non ha senso.
  expect(conflictReply("cancel", true)).toEqual({ type: "conflict", decision: { type: "cancel" }, applyToAll: false });
});

const withState = (id: number, type: JobView["state"]["type"]): JobView => ({ ...job(id, null), state: { type } });

test("unfinishedJobs: queued, running, paused and waiting jobs block a silent exit", () => {
  const states = ["queued", "running", "paused", "waiting", "done", "failed", "cancelled"] as const;
  const jobs = Object.fromEntries(states.map((s, i) => [i + 1, withState(i + 1, s)]));
  expect(unfinishedJobs(jobs).map((j) => j.state.type)).toEqual(["queued", "running", "paused", "waiting"]);
  expect(unfinishedJobs({})).toEqual([]);
});

test("cancelAllAndWait cancels every unfinished job and waits for its final state, at most until the timeout", async () => {
  useJobs.setState({ jobs: { 1: withState(1, "running"), 2: withState(2, "waiting"), 3: withState(3, "done") } });
  vi.mocked(api.cancelJob).mockResolvedValue();
  const done = cancelAllAndWait(3000);
  await vi.waitFor(() => expect(api.cancelJob).toHaveBeenCalledTimes(2));
  expect(vi.mocked(api.cancelJob).mock.calls.map((c) => c[0])).toEqual([1, 2]);
  let settled = false;
  void done.then(() => (settled = true));
  useJobs.setState((s) => ({ jobs: { ...s.jobs, 1: withState(1, "cancelled") } }));
  await Promise.resolve();
  expect(settled).toBe(false); // il 2 non è ancora finito
  useJobs.setState((s) => ({ jobs: { ...s.jobs, 2: withState(2, "cancelled") } }));
  await done;

  vi.useFakeTimers();
  useJobs.setState({ jobs: { 4: withState(4, "running") } });
  const late = cancelAllAndWait(3000);
  await vi.advanceTimersByTimeAsync(3000); // un job che non risponde non blocca l'uscita
  await late;
  vi.useRealTimers();
});

test("waitForJob resolves as cancelled if the job is removed before its final event", async () => {
  useJobs.setState({ jobs: { 5: withState(5, "running") } });
  const w = waitForJob(5);
  useJobs.setState({ jobs: {} }); // clear_finished + refresh prima dell'evento finale
  expect((await w).state.type).toBe("cancelled");
});

test("renamePath keeps the folder of the suggested name and changes only the file name", () => {
  expect(renamePath("C:\\d\\f (2).txt", "g.txt")).toBe("C:\\d\\g.txt");
  expect(renamePath("C:\\f (2).txt", "g.txt")).toBe("C:\\g.txt");
  expect(renamePath("\\\\srv\\sh\\f (2)", "g")).toBe("\\\\srv\\sh\\g");
});

test("remoteVolumesOf keeps only the connections among a job's volumes", () => {
  expect(remoteVolumesOf({ ...job(1, null), volumes: ["c:", "sftp://w"] })).toEqual(["sftp://w"]);
});

test("remoteVolumesOf maps the volume of an archive on a server to its connection", () => {
  const vols = ["c:", "arc://sftp://w/rel/a.zip", "arc://c:", "sftp://w"];
  expect(remoteVolumesOf({ ...job(1, null), volumes: vols })).toEqual(["sftp://w"]);
});

test("a job that finishes reloads every open archive tab, even for a job on local volumes", async () => {
  let emit!: (j: JobView) => void;
  vi.mocked(api.listJobs).mockResolvedValue([]);
  vi.mocked(api.onJobUpdated).mockImplementation(async (cb) => ((emit = cb), () => {}));
  await initJobs();
  useLayout.setState({ tabs: { a: { id: "a", path: "arc://C:\\r\\a.zip/d" }, b: { id: "b", path: "C:\\" }, c: { id: "c", path: "sftp://w/" } } });
  const reload = vi.fn(async () => {});
  useTabViews.setState({ reload });
  emit({ ...withState(8, "done"), volumes: ["arc://c:\\r\\a.zip", "c:"] });
  expect(reload.mock.calls).toEqual([["a"]]);
});

test("a job refused before it starts shows its error in the active tab, not only in Operations", async () => {
  let emit!: (j: JobView) => void;
  vi.mocked(api.listJobs).mockResolvedValue([]);
  vi.mocked(api.onJobUpdated).mockImplementation(async (cb) => ((emit = cb), () => {}));
  await initJobs();
  useLayout.setState({ tabs: { t: { id: "t", path: "C:\\" } }, groups: [{ id: "g", tabIds: ["t"], activeTabId: "t" }], activeGroupId: "g" });
  const showNotice = vi.fn();
  useTabViews.setState({ reload: vi.fn(async () => {}), showNotice });
  const into = { code: "INTO_ITSELF", path: "C:\\rel\\a.zip", detail: null } as const;
  emit({ ...withState(10, "failed"), filesTotal: 0, errors: [into] });
  expect(showNotice).toHaveBeenCalledWith("t", into);
  // Un job fallito dopo aver lavorato degli elementi ha già mostrato i suoi errori nei prompt.
  showNotice.mockClear();
  emit({ ...withState(11, "failed"), filesTotal: 3, errors: [into] });
  expect(showNotice).not.toHaveBeenCalled();
});

test("a job that finishes reloads the tabs on its remote connections, once", async () => {
  let emit!: (j: JobView) => void;
  vi.mocked(api.listJobs).mockResolvedValue([]);
  vi.mocked(api.onJobUpdated).mockImplementation(async (cb) => ((emit = cb), () => {}));
  await initJobs();
  useLayout.setState({ tabs: { a: { id: "a", path: "sftp://w/x" }, b: { id: "b", path: "sftp://v/" }, c: { id: "c", path: "C:\\" } } });
  const reload = vi.fn(async () => {});
  useTabViews.setState({ reload });
  const vols = (type: JobView["state"]["type"]) => ({ ...withState(9, type), volumes: ["c:", "sftp://w"] });
  emit(vols("running"));
  expect(reload).not.toHaveBeenCalled();
  emit(vols("done"));
  emit(vols("done"));
  expect(reload.mock.calls).toEqual([["a"]]);
});
