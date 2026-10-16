import { useTranslation } from "react-i18next";
import { useStatusJob } from "../../state/jobs";
import { kindKey, progressOf } from "../panel/OperationsView";

// Selezione e spazio libero arrivano nei task successivi (Task 20 rifinisce anche il job).
export function StatusBar() {
  const { t } = useTranslation();
  const job = useStatusJob();
  const pct = job ? Math.round(progressOf(job) * 100) : 0;
  return (
    <footer className="mc-status">
      {job && (
        <span className="mc-status-job">
          {t(kindKey(job.kind))}
          <span className="mc-status-prog"><i style={{ width: `${pct}%` }} /></span>
          {pct}%
        </span>
      )}
    </footer>
  );
}
