import { parentPath } from "../state/tabView";

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

/** Cosa si può fare su una posizione: decide quali comandi chiamano il backend. */
export interface SourceCaps { write: boolean; watch: boolean; shell: boolean; thumbs: boolean; hash: boolean; scan: boolean; dirSize: boolean }

const caps = (v: boolean): SourceCaps => ({ write: v, watch: v, shell: v, thumbs: v, hash: v, scan: v, dirSize: v });

/** Locale: tutto; lista delle unità e HTTP: niente; SFTP e FTP: solo la scrittura. */
export function sourceCaps(p: string): SourceCaps {
  if (p === DRIVES) return caps(false);
  const r = parseRemote(p);
  return r ? { ...caps(false), write: !r.scheme.startsWith("http") } : caps(true);
}

export const joinPath = (dir: string, name: string): string => {
  const sep = isRemote(dir) ? "/" : "\\";
  return dir.endsWith(sep) ? dir + name : `${dir}${sep}${name}`;
};

/**
 * Percorso digitato a mano: `/` → `\`, niente barre finali salvo la radice (`C:` → `C:\`). Un indirizzo con schema
 * resta con `/`: se è remoto si riduce come `Loc`, altrimenti (un host vero) resta com'è scritto.
 */
export function normalizePath(input: string): string {
  const t = input.trim();
  if (/^[A-Za-z0-9+.-]{2,}:\/\//.test(t)) {
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
  let p = path;
  for (let up = parentPath(p); up !== null; up = parentPath(p)) {
    if (up.startsWith("\\\\") && parentPath(up) === null) break;
    p = up;
  }
  return p;
}

/** Segmenti del breadcrumb, dalla radice (`C:`, `\\server` o l'id della connessione) alla cartella corrente. */
export function pathCrumbs(path: string): { label: string; path: string }[] {
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

/** Cartella in cui entra Invio sulla riga, oppure null se la riga è un file. */
export function dirTarget(
  path: string,
  row: { name: string; isDir: boolean; isParent?: boolean; drive?: unknown },
): string | null {
  if (row.isParent) return parentPath(path);
  if (row.drive) return row.name;
  return row.isDir ? joinPath(path, row.name) : null;
}
