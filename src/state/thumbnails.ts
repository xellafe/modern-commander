// Coda delle miniature: concorrenza limitata, cache LRU, richieste annullabili per proprietario (la scheda).
import { thumbnail } from "../ipc/api";

interface Job<T> {
  path: string;
  size: number;
  started: boolean;
  waiters: { owner: string; resolve: (v: T | null) => void }[];
}

export interface ThumbQueue<T> {
  /** Il valore, o `null` se la richiesta è stata annullata o `fetch` ha fallito (un file senza miniatura non è un errore da mostrare). */
  request(owner: string, key: string, path: string, size: number): Promise<T | null>;
  /** Toglie dalla coda le richieste non ancora partite di `owner`; quelle in corso finiscono e restano in cache. */
  cancelOwner(owner: string): void;
  /** Come `cancelOwner`, ma per una sola chiave: il riquadro uscito dalla vista non ingolfa la coda. */
  release(owner: string, key: string): void;
}

/** Chiave di cache: lo stesso file modificato o chiesto a un'altra dimensione è un'altra miniatura. */
export const thumbKey = (path: string, modifiedMs: number, size: number) => `${path}|${modifiedMs}|${size}`;

export function createThumbQueue<T>(
  fetch: (path: string, size: number) => Promise<T>,
  opts: { concurrency: number; cacheSize: number; dispose(v: T): void },
): ThumbQueue<T> {
  const cache = new Map<string, T>(); // l'ordine di inserimento è l'ordine LRU
  const jobs = new Map<string, Job<T>>(); // in coda o in corso
  const queue: string[] = [];
  let running = 0;

  const finish = (key: string, value: T | null) => {
    const job = jobs.get(key)!;
    jobs.delete(key);
    running--;
    if (value !== null) {
      cache.set(key, value);
      while (cache.size > opts.cacheSize) {
        const [oldest, v] = cache.entries().next().value!;
        cache.delete(oldest);
        opts.dispose(v);
      }
    }
    for (const w of job.waiters) w.resolve(value);
    pump(); // prima che i chiamanti riprendano: la coda non resta mai ferma con posti liberi
  };

  const pump = () => {
    while (running < opts.concurrency && queue.length) {
      const key = queue.shift()!;
      const job = jobs.get(key)!;
      job.started = true;
      running++;
      let p: Promise<T>;
      try {
        p = fetch(job.path, job.size);
      } catch (e) {
        p = Promise.reject(e);
      }
      p.then((v) => finish(key, v), () => finish(key, null));
    }
  };

  const drop = (match: (key: string, owner: string) => boolean) => {
    for (const [key, job] of [...jobs]) {
      if (job.started) continue;
      job.waiters = job.waiters.filter((w) => {
        if (!match(key, w.owner)) return true;
        w.resolve(null);
        return false;
      });
      if (!job.waiters.length) {
        jobs.delete(key);
        queue.splice(queue.indexOf(key), 1);
      }
    }
  };

  return {
    request(owner, key, path, size) {
      const hit = cache.get(key);
      if (hit !== undefined) {
        cache.delete(key); // tornato in testa all'LRU
        cache.set(key, hit);
        return Promise.resolve(hit);
      }
      return new Promise<T | null>((resolve) => {
        let job = jobs.get(key);
        if (!job) {
          job = { path, size, started: false, waiters: [] };
          jobs.set(key, job);
          queue.push(key);
        }
        job.waiters.push({ owner, resolve });
        pump();
      });
    },
    cancelOwner: (owner) => drop((_, o) => o === owner),
    release: (owner, key) => drop((k, o) => o === owner && k === key),
  };
}

/** Lato in pixel del riquadro nella griglia: tutte le miniature dell'app nascono a questa dimensione. */
export const THUMB_SIZE = 96;

/** Le miniature della shell come `ImageBitmap`: 4 richieste alla volta, 400 in cache, il bitmap si chiude all'espulsione. */
export const thumbs = createThumbQueue<ImageBitmap>(
  async (path, size) => {
    const { width, height, data } = await thumbnail(path, size);
    return createImageBitmap(new ImageData(data as Uint8ClampedArray<ArrayBuffer>, width, height));
  },
  { concurrency: 4, cacheSize: 400, dispose: (b) => b.close() },
);
