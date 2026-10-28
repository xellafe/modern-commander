import { Archive, Copy, FolderInput, FolderPlus, GripVertical, PencilLine, Play, Pause, Trash2, X, type LucideIcon } from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { cancelJob, clearFinished, moveJob, pauseJob, resumeJob } from "../../ipc/api";
import type { JobKind, JobView } from "../../ipc/types";
import { formatEta, formatSize } from "../../lib/format";
import { isFinished, kindKey, newestJob, progressOf, useJobs } from "../../state/jobs";
import "./panel.css";

const ICONS: Record<JobKind["type"], LucideIcon> = {
  copy: Copy, move: FolderInput, delete: Trash2, mkdir: FolderPlus, rename: PencilLine, pack: Archive,
};

const refresh = () => useJobs.getState().refresh().catch(console.error);

/**
 * Coda delle operazioni: avanzamento, velocità, ETA, pausa/riprendi/annulla, riordino col trascinamento.
 * Un job nuovo si porta in vista; se la scheda è nascosta, la prima volta che si mostra.
 */
export function OperationsView({ visible }: { visible: boolean }) {
  const { t } = useTranslation();
  const order = useJobs((s) => s.order);
  const jobs = useJobs((s) => s.jobs);
  const list = useRef<HTMLDivElement>(null);
  const seen = useRef(0);

  useEffect(() => {
    if (!visible) return;
    const id = newestJob(order, seen.current);
    if (id === null) return;
    seen.current = id;
    list.current?.querySelector(`[data-job-id="${id}"]`)?.scrollIntoView({ block: "nearest" });
  }, [order, visible]);
  // Trascinamento in corso: job e posizione d'arrivo (indice nella coda senza il job trascinato).
  const [drag, setDrag] = useState<{ id: number; to: number } | null>(null);

  const startDrag = (e: PointerEvent<HTMLElement>, id: number) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const indexAt = (y: number) => {
      const rows = [...list.current!.querySelectorAll<HTMLElement>("[data-job-id]")]
        .filter((r) => r.dataset.jobId !== String(id));
      return rows.filter((r) => { const b = r.getBoundingClientRect(); return b.top + b.height / 2 < y; }).length;
    };
    let to = order.indexOf(id);
    setDrag({ id, to });
    const move = (ev: globalThis.PointerEvent) => {
      to = indexAt(ev.clientY);
      setDrag({ id, to });
    };
    const stop = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", stop);
      el.removeEventListener("pointercancel", stop);
      setDrag(null);
      // move_job non emette eventi: l'ordine si rilegge.
      if (to !== order.indexOf(id)) moveJob(id, to).then(refresh, console.error);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", stop);
    el.addEventListener("pointercancel", stop);
  };

  // Posizione di arrivo in coordinate della lista completa (per il segno di inserimento).
  const shown = drag ? order.filter((id) => id !== drag.id) : order;
  const dropBefore = drag ? (shown[drag.to] ?? null) : undefined;

  return (
    <div className="mc-ops">
      <div className="mc-ops-bar">
        <button
          title={t("ops.clearFinished")} aria-label={t("ops.clearFinished")}
          onClick={() => clearFinished().then(refresh, console.error)} disabled={!order.some((id) => jobs[id] && isFinished(jobs[id]))}
        >
          <Trash2 size={13} />
        </button>
      </div>
      {order.length === 0 && <div className="mc-ops-empty">{t("ops.empty")}</div>}
      <div ref={list}>
        {order.map((id) => jobs[id] && (
          <JobRow key={id} job={jobs[id]} dragging={drag?.id === id} dropBefore={dropBefore === id} onGrip={(e) => startDrag(e, id)} />
        ))}
        {drag && dropBefore === null && <div className="mc-op-drop" />}
      </div>
    </div>
  );
}

function JobRow({ job: j, dragging, dropBefore, onGrip }: {
  job: JobView; dragging: boolean; dropBefore: boolean; onGrip: (e: PointerEvent<HTMLElement>) => void;
}) {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;
  const Icon = ICONS[j.kind.type];
  const finished = isFinished(j);
  const pct = Math.round(progressOf(j) * 100);
  const state = j.state.type;
  const meta = [
    t("ops.filesProgress", { done: j.filesDone, total: j.filesTotal }),
    j.bytesTotal > 0 && `${formatSize(j.bytesDone, lng)} / ${formatSize(j.bytesTotal, lng)}`,
    state === "running" && j.speedBps > 0 && t("ops.speed", { size: formatSize(j.speedBps, lng) }),
    state === "running" && j.speedBps > 0 && j.bytesTotal > j.bytesDone &&
      t("ops.eta", { time: formatEta((j.bytesTotal - j.bytesDone) / j.speedBps) }),
  ].filter(Boolean).join(" · ");

  return (
    <>
      {dropBefore && <div className="mc-op-drop" />}
      <div className={`mc-op ${state}${dragging ? " dragging" : ""}`} data-job-id={j.id}>
        <span className="mc-op-grip" title={t("ops.reorder")} onPointerDown={onGrip}><GripVertical size={12} aria-hidden /></span>
        <Icon size={14} className="mc-op-icon" aria-hidden />
        <div className="mc-op-main">
          <div className="mc-op-line">
            <b>{t(kindKey(j.kind))}</b>
            <span className="mc-op-state">{t(`ops.state.${state}`)}</span>
            <span className="mc-op-current" title={j.current ?? undefined}>{j.current}</span>
            <span className="mc-op-pct">{pct}%</span>
          </div>
          <div className="mc-op-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
            <i style={{ width: `${pct}%` }} />
          </div>
          <div className="mc-op-meta">{meta}</div>
          {j.errors.length > 0 && (
            <details className="mc-op-errors">
              <summary>{t("ops.errorCount", { count: j.errors.length })}</summary>
              <ul>
                {j.errors.map((e, i) => (
                  <li key={i}>{t(`errors.${e.code}`)} <span className="mc-path">{e.path}</span></li>
                ))}
              </ul>
            </details>
          )}
        </div>
        {!finished && (
          <div className="mc-op-actions">
            {state === "paused"
              ? <button title={t("ops.resume")} aria-label={t("ops.resume")} onClick={() => resumeJob(j.id).catch(console.error)}><Play size={13} /></button>
              : <button title={t("ops.pause")} aria-label={t("ops.pause")} onClick={() => pauseJob(j.id).catch(console.error)}><Pause size={13} /></button>}
            <button title={t("ops.cancelJob")} aria-label={t("ops.cancelJob")} onClick={() => cancelJob(j.id).catch(console.error)}><X size={13} /></button>
          </div>
        )}
      </div>
    </>
  );
}
