import { create } from "zustand";
import { connect, deleteConnection, listConnections, saveConnection, trustHost } from "../ipc/api";
import type { AppError, Connection, ConnectionView, ErrorCode, Scheme } from "../ipc/types";
import { normalizePath, parseRemote } from "../lib/paths";
import { useUi, type DialogRequest } from "./ui";

interface ConnectionsState {
  list: ConnectionView[];
  load: () => Promise<void>;
  /** Un `id` vuoto crea una connessione nuova; la password si salva solo con `remember`. */
  save: (conn: Connection, password: string | null, remember: boolean) => Promise<ConnectionView>;
  /** Elimina anche password e impronta. */
  remove: (id: string) => Promise<void>;
  /** Nome mostrato; l'id stesso se la connessione non c'è (più). */
  nameOf: (id: string) => string;
}

export const useConnections = create<ConnectionsState>((set, get) => ({
  list: [],
  load: async () => set({ list: await listConnections() }),
  save: async (conn, password, remember) => {
    const view = await saveConnection(conn, password, remember);
    set((s) => ({
      list: s.list.some((c) => c.id === view.id) ? s.list.map((c) => (c.id === view.id ? view : c)) : [...s.list, view],
    }));
    return view;
  },
  remove: async (id) => {
    await deleteConnection(id);
    set((s) => ({ list: s.list.filter((c) => c.id !== id) }));
  },
  nameOf: (id) => get().list.find((c) => c.id === id)?.name || id,
}));

export const defaultPort = (scheme: Scheme, ftpsImplicit = false): number =>
  ({ sftp: 22, ftp: 21, ftps: ftpsImplicit ? 990 : 21, http: 80, https: 443 })[scheme];

/** Posizione della cartella iniziale della connessione. */
export const startLocation = (c: Connection) => normalizePath(`${c.scheme}://${c.id}/${c.startDir}`);

const TYPED = /^(sftp|ftps?|https?):\/\/([^/]*)(\/[\s\S]*)?$/i;

/** Ha la forma di un indirizzo remoto scritto a mano (anche se poi non si risolve). */
export const isTypedAddress = (input: string) => TYPED.test(input.trim());

/**
 * Host accettato dal backend (`valid_host`): nome, IPv4 o IPv6 senza parentesi; niente spazi, controlli né
 * `/ \ ? # @ [ ]`.
 */
export const validHost = (host: string) =>
  !!host && ![...host].some((c) => c <= " " || c === "\x7f" || /\s/.test(c) || "/\\?#@[]".includes(c));

/** Host IPv6 senza parentesi quadre (`[::1]` → `::1`). */
const bare = (host: string) => host.replace(/^\[(.*)\]$/, "$1");

/**
 * Indirizzo scritto a mano: `<schema>://[utente@]host[:porta]/percorso` si apre sulla connessione salvata con stesso
 * schema, host e porta (di default quella dello schema) e, se indicato, stesso utente; senza corrispondenza dà la
 * connessione nuova da compilare. Un id di connessione dopo `://` si apre com'è. Null se non è un indirizzo remoto.
 */
export function resolveTypedAddress(
  input: string,
  list: ConnectionView[],
): { open: string } | { create: Partial<Connection> } | null {
  const m = TYPED.exec(input.trim());
  if (!m) return null;
  const scheme = m[1].toLowerCase() as Scheme;
  const auth = m[2];
  // Percorso ripulito come in `Loc` (`/` doppi, `.`, `..`).
  const path = parseRemote(`${scheme}://_${m[3] ?? "/"}`)!.path;
  const byId = list.find((c) => c.id === auth);
  if (byId) return { open: `${byId.scheme}://${byId.id}${path}` };

  // L'utente può contenere `@` (un indirizzo email): l'host è dopo l'ultima. Una password (`utente:password@`) non
  // entra mai nella connessione: si chiede alla connessione e si salva solo nel Credential Manager.
  const at = auth.lastIndexOf("@");
  const user = at >= 0 ? auth.slice(0, at).split(":")[0] : null;
  const hp = /^(.+?)(?::(\d{1,5}))?$/.exec(auth.slice(at + 1));
  if (!hp) return null;
  const host = bare(hp[1]);
  if (!validHost(host)) return null;
  const port = hp[2] ? Number(hp[2]) : defaultPort(scheme);
  const match = list.find((c) =>
    c.scheme === scheme && bare(c.host).toLowerCase() === host.toLowerCase() && c.port === port && (user === null || c.user === user));
  if (match) return { open: `${scheme}://${match.id}${path}` };
  return {
    create: { name: host, scheme, host, port, user: user ?? "", startDir: path, ftpsImplicit: scheme === "ftps" && port === 990 },
  };
}

const AUTH: ErrorCode[] = ["AUTH_REQUIRED", "AUTH_FAILED"];
const TRUST: ErrorCode[] = ["HOST_KEY_UNKNOWN", "CERT_UNTRUSTED"];

/**
 * Apre una finestra e ne aspetta la risposta; con un'altra finestra già aperta risponde subito `none`.
 * ponytail: all'avvio la domanda di una seconda connessione, mentre è aperta quella della prima, vale come Annulla
 * (la scheda resta vuota, Ctrl+R la ripropone); una coda di domande se diventa scomodo.
 */
function ask<T>(none: T, make: (done: (v: T) => void) => DialogRequest): Promise<T> {
  return new Promise((resolve) => {
    if (useUi.getState().dialog) return resolve(none);
    useUi.getState().openDialog(make((v) => {
      useUi.getState().closeDialog();
      resolve(v);
    }));
  });
}

const message = (title: string, text: string, values?: Record<string, string>) =>
  useUi.getState().openDialog({ kind: "message", title, text, values });

// Più schede sulla stessa connessione (all'avvio) fanno una domanda sola.
const pending = new Map<string, Promise<boolean>>();

/**
 * Dopo un errore di connessione chiede quello che serve (password, conferma dell'impronta) e riconnette.
 * `true` se ora la connessione c'è; `false` se l'utente annulla o l'errore non si risolve qui (impronta cambiata:
 * solo un avviso). Un altro errore di `connect` (server irraggiungibile…) si rilancia.
 */
export async function ensureConnected(id: string, error: AppError): Promise<boolean> {
  let p = pending.get(id);
  if (!p) {
    p = settle(id, error).finally(() => pending.delete(id));
    pending.set(id, p);
  }
  return p;
}

async function settle(id: string, first: AppError): Promise<boolean> {
  let err = first;
  // Restano per il nuovo tentativo dopo l'impronta: la password scritta non si richiede.
  let password: string | null = null;
  let remember = false;
  for (;;) {
    const code = err?.code;
    if (AUTH.includes(code)) {
      const r = await ask<{ password: string; remember: boolean } | null>(null, (done) =>
        ({ kind: "password", id, failed: code === "AUTH_FAILED", done }));
      if (!r) return false;
      ({ password, remember } = r);
    } else if (TRUST.includes(code) && err.detail) {
      const fingerprint = err.detail;
      if (!(await ask<boolean>(false, (done) => ({ kind: "trust", id, tls: code === "CERT_UNTRUSTED", fingerprint, done })))) return false;
      await trustHost(id, fingerprint);
    } else {
      // Si sblocca solo dimenticando l'impronta vecchia nel gestore: mai accettare la nuova da qui.
      if (code === "HOST_KEY_CHANGED") {
        message("connections.keyChangedTitle", "connections.keyChanged", { name: useConnections.getState().nameOf(id), fingerprint: err.detail ?? "" });
      }
      return false;
    }
    try {
      if (!(await connect(id, password, remember)) && remember) message("connections.title", "connections.notRemembered");
      return true;
    } catch (e) {
      err = e as AppError;
      if (![...AUTH, ...TRUST, "HOST_KEY_CHANGED"].includes(err?.code)) throw e;
    }
  }
}
