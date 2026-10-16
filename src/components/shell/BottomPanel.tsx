import { useTranslation } from "react-i18next";
import { isFinished, useJobs } from "../../state/jobs";
import { useUi } from "../../state/ui";
import { OperationsView } from "../panel/OperationsView";
import { Splitter } from "./Splitter";

export function BottomPanel() {
  const { t } = useTranslation();
  const height = useUi((s) => s.bottomPanelHeight);
  const setHeight = useUi((s) => s.setBottomPanelHeight);
  const active = useJobs((s) => Object.values(s.jobs).filter((j) => !isFinished(j)).length);

  // In questa fase esiste la sola scheda Operazioni.
  return (
    <section className="mc-panel" style={{ height }}>
      <Splitter axis="y" size={height} onResize={setHeight} invert />
      <div className="mc-panel-head">
        <span className="on">{active ? t("shell.panel.operationsCount", { count: active }) : t("shell.panel.operations")}</span>
      </div>
      <div className="mc-panel-body">
        <OperationsView />
      </div>
    </section>
  );
}
