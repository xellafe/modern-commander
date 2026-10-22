import { parentPath } from "../state/tabView";

/** Pseudo-percorso della lista delle unità (quando una scheda non ha più una cartella valida). */
export const DRIVES = "drives:";

export const joinPath = (dir: string, name: string): string =>
  dir.endsWith("\\") ? dir + name : `${dir}\\${name}`;

/** Percorso digitato a mano: `/` → `\`, niente barre finali salvo la radice (`C:` → `C:\`). */
export function normalizePath(input: string): string {
  const p = input.trim().replace(/\//g, "\\");
  const unc = p.startsWith("\\\\");
  const body = (unc ? p.slice(2) : p).replace(/\\+/g, "\\").replace(/\\$/, "");
  if (unc) return `\\\\${body}`;
  return /^[A-Za-z]:$/.test(body) ? `${body}\\` : body;
}

export function rootPath(path: string): string {
  let p = path;
  for (let up = parentPath(p); up !== null; up = parentPath(p)) p = up;
  return p;
}

/** Segmenti del breadcrumb, dalla radice (`C:` o `\\server`) alla cartella corrente. */
export function pathCrumbs(path: string): { label: string; path: string }[] {
  const chain: string[] = [];
  for (let p: string | null = path; p !== null; p = parentPath(p)) chain.unshift(p);
  return chain.map((p, i) => ({
    label: i === 0 ? p.replace(/\\+$/, "") : p.slice(p.lastIndexOf("\\") + 1),
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
