import { useTranslation } from "react-i18next";

const KEYS = [
  ["F3", "view"],
  ["F4", "edit"],
  ["F5", "copy"],
  ["F6", "move"],
  ["F7", "newFolder"],
  ["F8", "delete"],
  ["Alt+F4", "exit"],
] as const;

export function FKeyBar() {
  const { t } = useTranslation();
  return (
    <footer className="mc-fkeys">
      {KEYS.map(([key, action]) => (
        <span key={key} className="mc-fkey">
          <b>{key}</b>
          {t(`shell.fkeys.${action}`)}
        </span>
      ))}
    </footer>
  );
}
