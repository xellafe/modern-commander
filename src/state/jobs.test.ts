import { expect, test } from "vitest";
import type { JobView, Prompt } from "../ipc/types";
import { conflictChoices, conflictReply, pendingPrompts, renamePath } from "./jobs";

test("conflictChoices on the same file offers only skip, rename and cancel", () => {
  expect(conflictChoices(true)).toEqual(["skip", "rename", "cancel"]);
  expect(conflictChoices(false)).toEqual(["overwrite", "skip", "rename", "newer", "larger", "cancel"]);
});

const job = (id: number, prompt: Prompt | null): JobView => ({
  id, kind: { type: "copy" }, state: { type: prompt ? "waiting" : "running" },
  filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, current: null, speedBps: 0,
  volumes: [], errors: [], prompt,
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

test("renamePath keeps the folder of the suggested name and changes only the file name", () => {
  expect(renamePath("C:\\d\\f (2).txt", "g.txt")).toBe("C:\\d\\g.txt");
  expect(renamePath("C:\\f (2).txt", "g.txt")).toBe("C:\\g.txt");
  expect(renamePath("\\\\srv\\sh\\f (2)", "g")).toBe("\\\\srv\\sh\\g");
});
