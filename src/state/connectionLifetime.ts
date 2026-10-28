// Chiude le connessioni che nessuna scheda e nessun job non finito usa più: con la connessione il backend dimentica
// anche la password non salvata, che alla prossima apertura si richiede.
import { disconnect } from "../ipc/api";
import type { JobId, JobView } from "../ipc/types";
import { connectionPath, parseRemote } from "../lib/paths";
import { isFinished, useJobs } from "./jobs";
import { useLayout, type Tab } from "./layout";

/** Id delle connessioni nelle schede (cartella, file, ritorno) e nei volumi dei job non finiti. */
export function connectionsInUse(tabs: Record<string, Tab>, jobs: Record<JobId, JobView>): Set<string> {
  const paths = [
    ...Object.values(tabs).flatMap((t) => [t.path, t.file?.path, t.returnTo]),
    ...Object.values(jobs).flatMap((j) => (isFinished(j) ? [] : j.volumes)),
  ];
  return new Set(paths.flatMap((p) => {
    // Un archivio usa la connessione del suo contenitore; nei volumi di un job è `arc://<contenitore>`, non codificato.
    const r = p ? parseRemote(connectionPath(p).replace(/^arc:\/\//i, "")) : null;
    return r ? [r.conn] : [];
  }));
}

/** Da chiamare una volta all'avvio; restituisce la funzione che smette di osservare. */
export function initConnectionLifetime(): () => void {
  let { tabs } = useLayout.getState();
  let { jobs } = useJobs.getState();
  let inUse = connectionsInUse(tabs, jobs);
  const check = () => {
    // Si ricalcola solo se schede o job sono cambiati, non per il gruppo o la scheda attiva.
    if (useLayout.getState().tabs === tabs && useJobs.getState().jobs === jobs) return;
    ({ tabs } = useLayout.getState());
    ({ jobs } = useJobs.getState());
    const now = connectionsInUse(tabs, jobs);
    for (const id of inUse) if (!now.has(id)) disconnect(id).catch(console.error);
    inUse = now;
  };
  const offLayout = useLayout.subscribe(check);
  const offJobs = useJobs.subscribe(check);
  return () => {
    offLayout();
    offJobs();
  };
}
