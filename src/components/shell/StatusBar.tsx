import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { formatSize } from "../../lib/format";
import { DRIVES, folderName } from "../../lib/paths";
import { driveOf, summarize, summaryText } from "../../lib/status";
import { useDrives, useDrivesPolling } from "../../state/drives";
import { activeJobs, kindKey, progressOf, useJobs } from "../../state/jobs";
import { useLayout } from "../../state/layout";
import { useTabViews } from "../../state/tabViews";
import { useUi } from "../../state/ui";

// Il git branch resta nascosto fino alla fase 5.
export function StatusBar() {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;

  const tabId = useLayout((s) => s.groups.find((g) => g.id === s.activeGroupId)?.activeTabId);
  const path = useLayout((s) => (tabId ? s.tabs[tabId]?.path : undefined)) ?? DRIVES;
  const view = useTabViews((s) => (tabId ? s.views[tabId] : undefined));
  const drives = useDrives((s) => s.drives);
  const order = useJobs((s) => s.order);
  const jobs = useJobs((s) => s.jobs);

  useDrivesPolling(); // unico punto di lettura delle unità (la sidebar legge solo lo store)

  // Il cursore non cambia rows/marked/dirSizes: spostarlo non rifà il conteggio (anche su 100.000 righe).
  const summary = useMemo(
    () => (view && path !== DRIVES ? summarize(view) : null),
    [view?.rows, view?.marked, view?.dirSizes, path], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const running = useMemo(() => activeJobs(order, jobs), [order, jobs]);
  const job = running[0];
  const pct = job ? Math.round(progressOf(job) * 100) : 0;
  const drive = driveOf(path, drives);

  return (
    <footer className="mc-status">
      {summary && <span>{summaryText(summary, t, lng)}</span>}
      {job && (
        <button
          type="button"
          className="mc-status-job"
          title={t("shell.panel.operations")}
          onClick={() => useUi.setState({ bottomPanelVisible: true, bottomPanelTab: "operations" })}
        >
          {t(kindKey(job.kind))}
          {job.dest && <span className="mc-status-dest" title={job.dest}>{` → ${folderName(job.dest)}`}</span>}
          <span className="mc-status-prog"><i style={{ width: `${pct}%` }} /></span>
          {pct}%
          {running.length > 1 && ` +${running.length - 1}`}
        </button>
      )}
      {drive && (
        <span className="mc-status-free">
          {t("status.free", { drive: drive.root.replace(/\\$/, ""), size: formatSize(drive.free, lng) })}
        </span>
      )}
    </footer>
  );
}
