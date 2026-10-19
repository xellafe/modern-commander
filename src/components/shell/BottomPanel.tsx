import { useTranslation } from "react-i18next";
import { showTerminalTab } from "../../commands/terminal";
import { isFinished, useJobs } from "../../state/jobs";
import { useUi } from "../../state/ui";
import { OperationsView } from "../panel/OperationsView";
import { TerminalView } from "../panel/TerminalView";
import { Splitter } from "./Splitter";

/** Sempre montato: un'istanza xterm non si riapre su un altro elemento, quindi il pannello si nasconde con `hidden`. */
export function BottomPanel({ hidden }: { hidden: boolean }) {
  const { t } = useTranslation();
  const height = useUi((s) => s.bottomPanelHeight);
  const setHeight = useUi((s) => s.setBottomPanelHeight);
  const tab = useUi((s) => s.bottomPanelTab);
  const active = useJobs((s) => Object.values(s.jobs).filter((j) => !isFinished(j)).length);

  return (
    <section className="mc-panel" style={{ height }} hidden={hidden}>
      <Splitter axis="y" size={height} onResize={setHeight} invert />
      <div className="mc-panel-head" role="tablist">
        <button role="tab" aria-selected={tab === "terminal"} className={tab === "terminal" ? "on" : ""} onClick={showTerminalTab}>
          {t("shell.panel.terminal")}
        </button>
        <button role="tab" aria-selected={tab === "operations"} className={tab === "operations" ? "on" : ""} onClick={() => useUi.getState().showBottomPanel("operations")}>
          {active ? t("shell.panel.operationsCount", { count: active }) : t("shell.panel.operations")}
        </button>
      </div>
      <div className="mc-panel-body" hidden={tab !== "operations"}>
        <OperationsView />
      </div>
      <div className="mc-panel-body mc-panel-term" hidden={tab !== "terminal"}>
        <TerminalView />
      </div>
    </section>
  );
}
