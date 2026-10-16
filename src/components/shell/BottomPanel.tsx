import { useTranslation } from "react-i18next";
import { useUi } from "../../state/ui";
import { Splitter } from "./Splitter";

export function BottomPanel() {
  const { t } = useTranslation();
  const height = useUi((s) => s.bottomPanelHeight);
  const setHeight = useUi((s) => s.setBottomPanelHeight);

  // In questa fase esiste la sola scheda Operazioni.
  return (
    <section className="mc-panel" style={{ height }}>
      <Splitter axis="y" size={height} onResize={setHeight} invert />
      <div className="mc-panel-head">
        <span className="on">{t("shell.panel.operations")}</span>
      </div>
      <div className="mc-panel-body" />
    </section>
  );
}
