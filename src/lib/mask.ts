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

export function matchMask(name: string, mask: string): boolean {
  const bar = mask.indexOf("|");
  const include = patterns(bar < 0 ? mask : mask.slice(0, bar));
  const exclude = bar < 0 ? [] : patterns(mask.slice(bar + 1));
  if (exclude.some((r) => r.test(name))) return false;
  return include.length === 0 || include.some((r) => r.test(name));
}
