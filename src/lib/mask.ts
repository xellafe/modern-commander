// Maschere alla Total Commander: "inc1;inc2 inc3|exc1;exc2", case-insensitive, `*` e `?`.

const compile = (pattern: string): RegExp => {
  // `*.*` in TC combacia anche con i nomi senza estensione: equivale a `*`.
  const p = pattern === "*.*" ? "*" : pattern;
  const body = p
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*/g, ".*")
    .replace(/\?/g, ".");
  return new RegExp(`^${body}$`, "is");
};

const patterns = (list: string): RegExp[] =>
  list.split(/[; ]+/).filter(Boolean).map(compile);

/** Compila la maschera una volta sola: da usare quando si filtrano molti nomi. */
export function compileMask(mask: string): (name: string) => boolean {
  const bar = mask.indexOf("|");
  const include = patterns(bar < 0 ? mask : mask.slice(0, bar));
  const exclude = bar < 0 ? [] : patterns(mask.slice(bar + 1));
  return (name) =>
    !exclude.some((r) => r.test(name)) && (include.length === 0 || include.some((r) => r.test(name)));
}

export const matchMask = (name: string, mask: string): boolean => compileMask(mask)(name);

/**
 * Filtro rapido (Ctrl+S): un testo senza jolly cerca sottostringhe, come in TC.
 * Ogni parola (separata da spazio o `;`) è una sottostringa a sé, in OR come nelle maschere TC:
 * "my file" → `*my*;*file*`, così "my file.txt" resta visibile.
 */
export const quickFilterMask = (text: string): string =>
  !text || /[*?|]/.test(text)
    ? text
    : text.split(/[; ]+/).filter(Boolean).map((t) => `*${t}*`).join(";");
