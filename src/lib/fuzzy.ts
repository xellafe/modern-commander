// Ricerca fuzzy per la palette e il salto rapido: sottosequenza senza distinzione di maiuscole,
// con premio per inizi di parola e caratteri consecutivi. Un solo passaggio in avanti sul testo,
// nessuna regex: costa poco anche su decine di migliaia di voci.

export interface FuzzyMatch {
  score: number;
  /** Indici (UTF-16) dei caratteri del testo che corrispondono alla query, in ordine. */
  positions: number[];
}

const SEPARATORS = '\\/_-. ';

const isLower = (c: string) => c === c.toLowerCase() && c !== c.toUpperCase();
const isUpper = (c: string) => c === c.toUpperCase() && c !== c.toLowerCase();

/** Inizio del testo, dopo `\ / _ - .` o spazio, o maiuscola dopo minuscola. */
const isWordStart = (text: string, i: number): boolean =>
  i === 0 || SEPARATORS.includes(text[i - 1]) || (isUpper(text[i]) && isLower(text[i - 1]));

/** Posizioni scelte per ogni carattere di `q`; con `wordStarts` ogni carattere prova prima un inizio di parola. */
function place(q: string, lower: string, text: string, wordStarts: boolean): number[] | null {
  const positions: number[] = [];
  let from = 0;
  for (let k = 0; k < q.length; k++) {
    const c = q[k];
    let at = -1;
    if (wordStarts) {
      for (let i = from; i < lower.length; i++) {
        if (lower[i] === c && isWordStart(text, i)) { at = i; break; }
      }
    }
    if (at < 0) at = lower.indexOf(c, from);
    if (at < 0) return null;
    positions.push(at);
    from = at + 1;
  }
  return positions;
}

function score(positions: number[], text: string): number {
  let s = 0;
  positions.forEach((p, k) => {
    s += 1;
    if (isWordStart(text, p)) s += 8;
    if (k > 0 && p === positions[k - 1] + 1) s += 5;
  });
  return positions[0] === 0 ? s + 10 : s;
}

export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  if (!query) return { score: 0, positions: [] };
  const q = query.toLowerCase();
  let lower = text.toLowerCase();
  if (lower.length !== text.length) lower = text; // rarissimo (es. 'İ'): gli indici devono restare quelli del testo
  // Se l'inizio di parola scelto esaurisce il testo prima dell'ultimo carattere, si riprova con la sottosequenza semplice.
  const positions = place(q, lower, text, true) ?? place(q, lower, text, false);
  return positions && { score: score(positions, text), positions };
}

/** Le voci che corrispondono, dal punteggio più alto; a parità resta l'ordine di ingresso. Query vuota: le prime `limit`. */
export function fuzzyFilter<T>(items: T[], query: string, key: (t: T) => string, limit: number): { item: T; match: FuzzyMatch }[] {
  if (!query) return items.slice(0, limit).map((item) => ({ item, match: { score: 0, positions: [] } }));
  const hits: { item: T; match: FuzzyMatch }[] = [];
  for (const item of items) {
    const match = fuzzyMatch(query, key(item));
    if (match) hits.push({ item, match });
  }
  // Array.prototype.sort è stabile (ES2019): i pari merito restano nell'ordine di ingresso.
  return hits.sort((a, b) => b.match.score - a.match.score).slice(0, limit);
}

/** Il testo spezzato in tratti trovati / non trovati, per evidenziarli senza mai produrre HTML. */
export function highlightRuns(text: string, positions: number[]): { text: string; hit: boolean }[] {
  const runs: { text: string; hit: boolean }[] = [];
  const hits = new Set(positions);
  for (let i = 0; i < text.length; i++) {
    const hit = hits.has(i);
    const last = runs[runs.length - 1];
    if (last?.hit === hit) last.text += text[i];
    else runs.push({ text: text[i], hit });
  }
  return runs;
}
