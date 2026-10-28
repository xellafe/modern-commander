import { afterEach, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { JobView } from "../ipc/types";
import { connectionsInUse, initConnectionLifetime } from "./connectionLifetime";
import { useJobs } from "./jobs";
import { useLayout, type Tab } from "./layout";

const disconnect = vi.mocked(api.disconnect);

const tab = (id: string, path: string, extra: Partial<Tab> = {}): Tab => ({ id, path, ...extra });
const job = (id: number, state: JobView["state"]["type"], volumes: string[]): JobView => ({
  id, kind: { type: "copy" }, state: { type: state } as JobView["state"],
  filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, current: null, speedBps: 0,
  volumes, dest: null, errors: [], prompt: null,
});

let stop: () => void;
beforeEach(() => {
  vi.resetAllMocks();
  disconnect.mockResolvedValue(undefined);
  useJobs.setState({ jobs: {}, order: [] });
  useLayout.setState({
    tabs: { a: tab("a", "sftp://c1/var/log"), b: tab("b", "sftp://c1/tmp"), c: tab("c", "C:\\") },
  });
  stop = initConnectionLifetime();
});
afterEach(() => stop());

const closeTab = (id: string) => useLayout.setState((s) => ({ tabs: Object.fromEntries(Object.entries(s.tabs).filter(([k]) => k !== id)) }));

test("connectionsInUse: folder, file and return paths of the tabs, and volumes of unfinished jobs", () => {
  const tabs = {
    a: tab("a", "sftp://c1/x"),
    b: tab("b", "C:\\", { file: { path: "ftp://c2/f.txt", mode: "viewer" }, returnTo: "https://c3/" }),
  };
  const jobs = { 1: job(1, "running", ["C:", "sftp://c4"]), 2: job(2, "done", ["sftp://c5"]) };
  expect([...connectionsInUse(tabs, jobs)].sort()).toEqual(["c1", "c2", "c3", "c4"]);
});

test("connectionsInUse counts the remote container of an open archive, in tabs and in job volumes", () => {
  const tabs = {
    a: tab("a", "arc://sftp:%2F%2Fc1%2Fr%2Fa.zip/d"),
    b: tab("b", "C:\\", { file: { path: "arc://ftp:%2F%2Fc2%2Fa.zip/f.txt", mode: "viewer" } }),
    c: tab("c", "arc://C:\\r\\local.zip/"),
  };
  // Il volume di un job su un archivio è `arc://<contenitore>` non codificato.
  const jobs = { 1: job(1, "running", ["arc://sftp://c3/x/a.zip", "arc://c:\\r\\local.zip"]) };
  expect([...connectionsInUse(tabs, jobs)].sort()).toEqual(["c1", "c2", "c3"]);
});

test("closing the last tab of a connection disconnects it, not before", () => {
  closeTab("a");
  expect(disconnect).not.toHaveBeenCalled();
  closeTab("b");
  expect(disconnect).toHaveBeenCalledExactlyOnceWith("c1");
});

test("navigating the last tab away from the connection disconnects it", () => {
  closeTab("a");
  useLayout.setState((s) => ({ tabs: { ...s.tabs, b: tab("b", "sftp://c2/") } }));
  expect(disconnect).toHaveBeenCalledExactlyOnceWith("c1");
});

test("an unfinished job keeps the connection until it ends", () => {
  useJobs.setState({ jobs: { 1: job(1, "running", ["sftp://c1", "C:"]) }, order: [1] });
  closeTab("a");
  closeTab("b");
  expect(disconnect).not.toHaveBeenCalled();
  useJobs.setState({ jobs: { 1: job(1, "done", ["sftp://c1", "C:"]) } });
  expect(disconnect).toHaveBeenCalledExactlyOnceWith("c1");
});

test("a connection used again before closing is not disconnected", () => {
  closeTab("a");
  useLayout.setState((s) => ({ tabs: { ...s.tabs, d: tab("d", "sftp://c1/") } }));
  closeTab("b");
  expect(disconnect).not.toHaveBeenCalled();
});
