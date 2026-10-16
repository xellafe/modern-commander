// Testi e calcoli della barra di stato (puri, senza stato).
import type { TFunction } from "i18next";
import type { Drive } from "../ipc/types";
import { hasSize, type TabView } from "../state/tabView";
import { formatSize } from "./format";

export type Summary =
  /** `approx`: ci sono cartelle marcate di cui la dimensione non è nota, il totale è un minimo. */
  | { kind: "marked"; count: number; bytes: number; approx: boolean }
  | { kind: "contents"; files: number; folders: number };

/** Marcati con dimensione totale (file + cartelle già misurate) oppure, senza marcati, file e cartelle della vista (`..` esclusa). */
export function summarize(v: Pick<TabView, "rows" | "marked" | "dirSizes">): Summary {
  if (v.marked.size) {
    let count = 0, bytes = 0, approx = false;
    for (const r of v.rows) {
      if (r.isParent || !v.marked.has(r.name)) continue;
      count++;
      if (!r.isDir) bytes += r.size;
      else if (hasSize(v.dirSizes, r.name)) bytes += v.dirSizes[r.name];
      else approx = true;
    }
    if (count) return { kind: "marked", count, bytes, approx };
  }
  let folders = 0, files = 0;
  for (const r of v.rows) {
    if (r.isParent) continue;
    if (r.isDir) folders++;
    else files++;
  }
  return { kind: "contents", files, folders };
}

export function summaryText(s: Summary, t: TFunction, lang: string): string {
  if (s.kind === "marked") {
    return t(s.approx ? "status.selectionApprox" : "status.selection", { count: s.count, size: formatSize(s.bytes, lang) });
  }
  return [t("status.files", { count: s.files }), t("status.folders", { count: s.folders })].join(", ");
}

/** Unità pronta che contiene il percorso (radice confrontata senza distinguere le maiuscole); null per UNC e percorsi senza lettera. */
export function driveOf(path: string, drives: Drive[]): Drive | null {
  const p = `${path.replace(/\\+$/, "")}\\`.toLowerCase();
  return drives.find((d) => d.ready && p.startsWith(d.root.toLowerCase())) ?? null;
}
