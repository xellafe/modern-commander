import { parentPath } from "../state/tabView";
import { previewKind } from "./preview";

/** Pseudo-percorso della lista delle unità (quando una scheda non ha più una cartella valida). */
export const DRIVES = "drives:";

export type Scheme = "sftp" | "ftp" | "ftps" | "http" | "https";

// Le regole di `Loc` (src-tauri/src/vfs/loc.rs): `<schema>://<id connessione>/<percorso POSIX>`.
const REMOTE = /^(sftp|ftps?|https?):\/\/([A-Za-z0-9_-]{1,64})(\/[\s\S]*)?$/i;

/** Percorso POSIX assoluto: `/` doppi ridotti, niente `/` finale, `.` tolti e `..` risolti senza salire oltre la radice. */
function cleanPosix(p: string): string {
  const parts: string[] = [];
  for (const s of p.split("/")) {
    if (s === "..") parts.pop();
    else if (s && s !== ".") parts.push(s);
  }
  return `/${parts.join("/")}`;
}

/** Indirizzo remoto scomposto, oppure null per un percorso locale (anche `C://x`) o un indirizzo non valido. */
export function parseRemote(p: string): { scheme: Scheme; conn: string; path: string } | null {
  const m = REMOTE.exec(p);
  return m ? { scheme: m[1].toLowerCase() as Scheme, conn: m[2], path: cleanPosix(m[3] ?? "/") } : null;
}

export const isRemote = (p: string) => parseRemote(p) !== null;

const formatRemote = (r: { scheme: Scheme; conn: string; path: string }) => `${r.scheme}://${r.conn}${r.path}`;

// Le regole di `Loc::Archive`: `arc://<contenitore>/<percorso POSIX>`, il contenitore con soli `%` e `/` codificati.
const ARCHIVE = /^arc:\/\/([^/]*)(\/[\s\S]*)?$/i;
const ESCAPE = /%(25|2[Ff])/g;

/** Inverso della codifica del contenitore: solo `%25` e `%2F`, ogni altro `%` rende l'indirizzo non valido. */
const decodeContainer = (s: string): string | null =>
  s.replace(ESCAPE, "").includes("%") ? null : s.replace(ESCAPE, (_, c: string) => (c === "25" ? "%" : "/"));

/**
 * Posizione dentro un archivio scomposta: il contenitore (un percorso locale o remoto con un nome di file, mai un altro
 * archivio) e il percorso POSIX interno. Null per ogni altro indirizzo, anche per un `arc://` che `Loc` rifiuterebbe.
 */
export function parseArchive(p: string): { container: string; inner: string } | null {
  const m = ARCHIVE.exec(p);
  const decoded = m && decodeContainer(m[1]);
  if (!m || !decoded || /^arc:\/\//i.test(decoded)) return null;
  const inner = cleanPosix(m[2] ?? "/");
  if (/^[A-Za-z0-9+.-]{2,}:\/\//.test(decoded)) {
    const r = parseRemote(decoded);
    return r && r.path !== "/" ? { container: formatRemote(r), inner } : null;
  }
  return parentPath(decoded) === null ? null : { container: decoded, inner }; // senza genitore (`C:\`) non ha nome di file
}

export const isArchive = (p: string) => parseArchive(p) !== null;

/** Remoto o dentro un archivio: percorsi con `/`, senza shell e senza watcher. */
export const isVirtual = (p: string) => isRemote(p) || isArchive(p);

/** Indirizzo di `inner` (default la radice) nell'archivio `container`, nella forma testuale di `Loc`. */
export function formatArchive(container: string, inner = "/"): string {
  const r = parseRemote(container);
  const c = r ? formatRemote(r) : container;
  return `arc://${c.replace(/%/g, "%25").replace(/\//g, "%2F")}${cleanPosix(inner)}`;
}

/** Posizione su cui serve una connessione: il contenitore per un archivio, `p` altrimenti. */
export const connectionPath = (p: string): string => parseArchive(p)?.container ?? p;

/** Nome di un file che si apre come cartella (stesso elenco dell'anteprima). */
export const isArchiveName = (name: string) => previewKind(name) === "archive";

/** Solo gli ZIP si riscrivono; 7z, TAR e TAR.GZ sono in sola lettura. */
export const isZipName = (name: string) => /\.zip$/i.test(name);

/** Cosa si può fare su una posizione: decide quali comandi chiamano il backend. */
export interface SourceCaps { write: boolean; watch: boolean; shell: boolean; thumbs: boolean; hash: boolean; scan: boolean; dirSize: boolean }

const caps = (v: boolean): SourceCaps => ({ write: v, watch: v, shell: v, thumbs: v, hash: v, scan: v, dirSize: v });

/** Locale: tutto; lista delle unità e HTTP: niente; SFTP, FTP e archivi ZIP su un contenitore scrivibile: solo la scrittura. */
export function sourceCaps(p: string): SourceCaps {
  if (p === DRIVES) return caps(false);
  const a = parseArchive(p);
  if (a) return { ...caps(false), write: isZipName(a.container) && sourceCaps(a.container).write };
  const r = parseRemote(p);
  return r ? { ...caps(false), write: !r.scheme.startsWith("http") } : caps(true);
}

export const joinPath = (dir: string, name: string): string => {
  const sep = isVirtual(dir) ? "/" : "\\";
  return dir.endsWith(sep) ? dir + name : `${dir}${sep}${name}`;
};

/**
 * Percorso digitato a mano: `/` → `\`, niente barre finali salvo la radice (`C:` → `C:\`). Un indirizzo con schema
 * resta con `/`: se è remoto si riduce come `Loc`, altrimenti (un host vero) resta com'è scritto.
 */
export function normalizePath(input: string): string {
  const t = input.trim();
  if (/^[A-Za-z0-9+.-]{2,}:\/\//.test(t)) {
    const a = parseArchive(t);
    if (a) return formatArchive(a.container, a.inner);
    const r = parseRemote(t);
    return r ? formatRemote(r) : t;
  }
  const p = t.replace(/\//g, "\\");
  const unc = p.startsWith("\\\\");
  const body = (unc ? p.slice(2) : p).replace(/\\+/g, "\\").replace(/\\$/, "");
  if (unc) return `\\\\${body}`;
  return /^[A-Za-z]:$/.test(body) ? `${body}\\` : body;
}

/** Radice dell'unità o della condivisione UNC (`\\server` da solo è l'elenco delle condivisioni, non una radice). */
export function rootPath(path: string): string {
  const a = parseArchive(path);
  if (a) return formatArchive(a.container); // la radice dell'archivio, non quella del disco del contenitore
  let p = path;
  for (let up = parentPath(p); up !== null; up = parentPath(p)) {
    if (up.startsWith("\\\\") && parentPath(up) === null) break;
    p = up;
  }
  return p;
}

/** Segmenti del breadcrumb, dalla radice (`C:`, `\\server` o l'id della connessione) alla cartella corrente. */
export function pathCrumbs(path: string): { label: string; path: string }[] {
  const a = parseArchive(path);
  if (a) {
    // I crumb del contenitore, con il file stesso che punta alla radice dell'archivio, e poi le cartelle interne.
    const crumbs = pathCrumbs(a.container);
    crumbs[crumbs.length - 1].path = formatArchive(a.container);
    let at = "";
    for (const label of a.inner.split("/").filter(Boolean)) crumbs.push({ label, path: formatArchive(a.container, (at += `/${label}`)) });
    return crumbs;
  }
  const r = parseRemote(path);
  const sep = r ? "/" : "\\";
  const chain: string[] = [];
  for (let p: string | null = r ? formatRemote(r) : path; p !== null; p = parentPath(p)) chain.unshift(p);
  return chain.map((p, i) => ({
    label: i > 0 ? p.slice(p.lastIndexOf(sep) + 1) : r ? r.conn : p.replace(/\\+$/, ""),
    path: p,
  }));
}

/** Nome breve di una cartella (etichetta della scheda). */
export function folderName(path: string): string {
  const crumbs = pathCrumbs(path);
  return crumbs[crumbs.length - 1].label;
}

/** Cartella in cui entra Invio sulla riga (un archivio compreso), oppure null se la riga è un file. */
export function dirTarget(
  path: string,
  row: { name: string; isDir: boolean; isParent?: boolean; drive?: unknown },
): string | null {
  if (row.isParent) return parentPath(path);
  if (row.drive) return row.name;
  if (row.isDir) return joinPath(path, row.name);
  // Un archivio si apre come una cartella, ma non dentro un altro archivio (niente annidamento).
  return path !== DRIVES && !isArchive(path) && isArchiveName(row.name) ? formatArchive(joinPath(path, row.name)) : null;
}
