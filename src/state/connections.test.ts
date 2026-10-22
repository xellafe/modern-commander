import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { ConnectionView } from "../ipc/types";
import { ensureConnected, isTypedAddress, resolveTypedAddress, useConnections, validHost } from "./connections";
import { useUi, type DialogRequest } from "./ui";

const conn = (id: string, scheme: ConnectionView["scheme"], host: string, port: number, user: string): ConnectionView => ({
  id, name: `n-${id}`, scheme, host, port, user, startDir: "/", ftpsImplicit: false, hasPassword: false,
});
const LIST = [conn("c1", "sftp", "srv", 22, "bob"), conn("c2", "sftp", "srv", 2222, "alice"), conn("h1", "https", "web", 443, "")];

const connect = vi.mocked(api.connect);
const trustHost = vi.mocked(api.trustHost);

/** Risponde alla prossima finestra aperta da `ensureConnected`. */
function answer(fn: (d: DialogRequest) => void) {
  const off = useUi.subscribe((s) => {
    if (!s.dialog) return;
    off();
    fn(s.dialog);
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  useUi.setState({ dialog: null });
  useConnections.setState({ list: LIST });
});

test("typed address with host matches a saved connection by scheme, host and default port", () => {
  expect(resolveTypedAddress("sftp://srv/var/log", LIST)).toEqual({ open: "sftp://c1/var/log" });
  expect(resolveTypedAddress("SFTP://SRV/a/", LIST)).toEqual({ open: "sftp://c1/a" });
  expect(resolveTypedAddress("https://web/logs/", LIST)).toEqual({ open: "https://h1/logs" });
  expect(resolveTypedAddress("sftp://alice@srv:2222/x", LIST)).toEqual({ open: "sftp://c2/x" });
  expect(resolveTypedAddress("sftp://bob@srv", LIST)).toEqual({ open: "sftp://c1/" });
});

test("typed address with a different port or user gives a prefilled new connection", () => {
  expect(resolveTypedAddress("sftp://srv:2200/x", LIST)).toEqual({
    create: { name: "srv", scheme: "sftp", host: "srv", port: 2200, user: "", startDir: "/x", ftpsImplicit: false },
  });
  expect(resolveTypedAddress("sftp://carl@srv/x", LIST)).toEqual({
    create: { name: "srv", scheme: "sftp", host: "srv", port: 22, user: "carl", startDir: "/x", ftpsImplicit: false },
  });
  // Utente con `@` (email): l'host è dopo l'ultima.
  expect(resolveTypedAddress("ftps://me@corp.it@files:990/", LIST)).toEqual({
    create: { name: "files", scheme: "ftps", host: "files", port: 990, user: "me@corp.it", startDir: "/", ftpsImplicit: true },
  });
});

test("an existing connection id after :// opens directly; local paths are not addresses", () => {
  expect(resolveTypedAddress("sftp://c2/tmp", LIST)).toEqual({ open: "sftp://c2/tmp" });
  expect(resolveTypedAddress("C:\\Windows", LIST)).toBeNull();
  expect(resolveTypedAddress("smb://srv/x", LIST)).toBeNull();
});

test("AUTH_REQUIRED asks the password, then connects with remember", async () => {
  connect.mockResolvedValue(true);
  answer((d) => {
    expect(d.kind).toBe("password");
    if (d.kind === "password") d.done({ password: "pw", remember: true });
  });
  expect(await ensureConnected("c1", { code: "AUTH_REQUIRED", path: null, detail: null })).toBe(true);
  expect(connect).toHaveBeenCalledWith("c1", "pw", true);
  expect(useUi.getState().dialog).toBeNull();
});

test("AUTH_FAILED after a password asks again; cancelling gives false", async () => {
  connect.mockRejectedValueOnce({ code: "AUTH_FAILED", path: null, detail: null });
  const off = useUi.subscribe(({ dialog: d }) => {
    if (d?.kind !== "password") return;
    if (!d.failed) return d.done({ password: "bad", remember: false });
    off();
    d.done(null);
  });
  expect(await ensureConnected("c1", { code: "AUTH_REQUIRED", path: null, detail: null })).toBe(false);
  expect(connect).toHaveBeenCalledTimes(1);
});

test("HOST_KEY_UNKNOWN with Accept trusts the fingerprint, then connects", async () => {
  connect.mockResolvedValue(true);
  trustHost.mockResolvedValue();
  answer((d) => {
    expect(d).toMatchObject({ kind: "trust", fingerprint: "SHA256:x", tls: false });
    if (d.kind === "trust") d.done(true);
  });
  expect(await ensureConnected("c1", { code: "HOST_KEY_UNKNOWN", path: null, detail: "SHA256:x" })).toBe(true);
  expect(trustHost).toHaveBeenCalledWith("c1", "SHA256:x");
  expect(connect).toHaveBeenCalledWith("c1", null, false);
});

test("CERT_UNTRUSTED refused: no trust, no connect", async () => {
  answer((d) => d.kind === "trust" && d.tls && d.done(false));
  expect(await ensureConnected("h1", { code: "CERT_UNTRUSTED", path: null, detail: "SHA256:y" })).toBe(false);
  expect(trustHost).not.toHaveBeenCalled();
  expect(connect).not.toHaveBeenCalled();
});

test("HOST_KEY_CHANGED never trusts: blocking message only", async () => {
  expect(await ensureConnected("c1", { code: "HOST_KEY_CHANGED", path: null, detail: "SHA256:z" })).toBe(false);
  expect(trustHost).not.toHaveBeenCalled();
  expect(connect).not.toHaveBeenCalled();
  expect(useUi.getState().dialog?.kind).toBe("message");
});

test("store keeps the list in sync and names connections", async () => {
  vi.mocked(api.saveConnection).mockResolvedValue({ ...LIST[0], name: "Prod" });
  await useConnections.getState().save(LIST[0], null, false);
  expect(useConnections.getState().nameOf("c1")).toBe("Prod");
  expect(useConnections.getState().nameOf("zz")).toBe("zz");
  vi.mocked(api.deleteConnection).mockResolvedValue();
  await useConnections.getState().remove("c1");
  expect(useConnections.getState().list.map((c) => c.id)).toEqual(["c2", "h1"]);
});

test("IPv6 literals match without brackets", () => {
  const list = [...LIST, conn("v6", "sftp", "::1", 22, "")];
  expect(resolveTypedAddress("sftp://[::1]/x", list)).toEqual({ open: "sftp://v6/x" });
  expect(resolveTypedAddress("sftp://[fe80::2]:2222/", list)).toMatchObject({ create: { host: "fe80::2", port: 2222 } });
});

test("a password in the typed address never reaches the connection", () => {
  const r = resolveTypedAddress("sftp://bob:segreto@server/x", LIST);
  expect(r).toEqual({
    create: { name: "server", scheme: "sftp", host: "server", port: 22, user: "bob", startDir: "/x", ftpsImplicit: false },
  });
  expect(JSON.stringify(r)).not.toContain("segreto");
  // L'utente prima dei `:` trova la connessione salvata.
  expect(resolveTypedAddress("sftp://bob:segreto@srv/x", LIST)).toEqual({ open: "sftp://c1/x" });
  expect(resolveTypedAddress("ftps://me@corp.it:pw@files/", LIST)).toMatchObject({ create: { user: "me@corp.it" } });
});

test("a host with spaces or URL characters is not resolved", () => {
  for (const a of ["sftp://my host/x", "http://a?b/", "http://a#b/", "http://a\\b/"]) expect(resolveTypedAddress(a, LIST)).toBeNull();
  expect(validHost("example.org") && validHost("10.0.0.1") && validHost("::1")).toBe(true);
  for (const h of ["", " h", "a/b", "a@b", "[::1]", "a\tb"]) expect(validHost(h)).toBe(false);
});

test("an address with no host is not resolved", () => {
  expect(resolveTypedAddress("sftp://bob@/x", LIST)).toBeNull();
  expect(isTypedAddress("sftp://bob@/x")).toBe(true);
  expect(isTypedAddress("C:\\x")).toBe(false);
});

test("password not stored: connected, with a warning", async () => {
  connect.mockResolvedValue(false);
  answer((d) => d.kind === "password" && d.done({ password: "pw", remember: true }));
  expect(await ensureConnected("c1", { code: "AUTH_REQUIRED", path: null, detail: null })).toBe(true);
  expect(useUi.getState().dialog).toMatchObject({ kind: "message", text: "connections.notRemembered" });
});

test("a connection error after the password is rethrown, never taken as success", async () => {
  connect.mockRejectedValue({ code: "ACCESS_DENIED", path: null, detail: null });
  answer((d) => d.kind === "password" && d.done({ password: "pw", remember: true }));
  await expect(ensureConnected("c1", { code: "AUTH_REQUIRED", path: null, detail: null })).rejects.toMatchObject({ code: "ACCESS_DENIED" });
  expect(useUi.getState().dialog).toBeNull();
});

test("after accepting the fingerprint the typed password is reused", async () => {
  connect.mockRejectedValueOnce({ code: "HOST_KEY_UNKNOWN", path: null, detail: "SHA256:x" }).mockResolvedValueOnce(true);
  trustHost.mockResolvedValue();
  const off = useUi.subscribe(({ dialog: d }) => {
    if (d?.kind === "password") d.done({ password: "pw", remember: true });
    else if (d?.kind === "trust") {
      off();
      d.done(true);
    }
  });
  expect(await ensureConnected("c1", { code: "AUTH_REQUIRED", path: null, detail: null })).toBe(true);
  expect(connect.mock.calls).toEqual([["c1", "pw", true], ["c1", "pw", true]]);
});
