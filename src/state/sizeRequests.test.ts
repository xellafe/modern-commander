import { expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import { computeDirSize } from "./sizeRequests";

test("se la registrazione dell'ascoltatore fallisce, la richiesta successiva riprova", async () => {
  let lastId = 0;
  vi.mocked(api.nextRequestId).mockImplementation(() => ++lastId);
  vi.mocked(api.calcDirSize).mockResolvedValue();
  let emit!: Parameters<typeof api.onDirSize>[0];
  vi.mocked(api.onDirSize)
    .mockRejectedValueOnce(new Error("listen fallita"))
    .mockImplementationOnce(async (cb) => {
      emit = cb;
      return () => {};
    });

  await expect(computeDirSize("t1", "C:\\x")).rejects.toThrow("listen fallita");
  expect(api.calcDirSize).not.toHaveBeenCalled();

  const p = computeDirSize("t1", "C:\\x");
  await vi.waitFor(() => expect(api.calcDirSize).toHaveBeenCalledTimes(1));
  const id = vi.mocked(api.calcDirSize).mock.calls[0][0];
  emit({ requestId: id, path: "C:\\x", result: { bytes: 4, files: 1, dirs: 0, skipped: 0 }, error: null });
  await expect(p).resolves.toEqual({ bytes: 4, files: 1, dirs: 0, skipped: 0 });
  expect(api.onDirSize).toHaveBeenCalledTimes(2);
});
