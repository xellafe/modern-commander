import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { formatSize } from "../../lib/format";
import { useDrives } from "../../state/drives";
import { FavoritesList } from "./FavoritesView";
import { FolderTree } from "./FolderTree";

const DRIVES_REFRESH_MS = 10_000;

export function ExplorerView() {
  const { t, i18n } = useTranslation();
  const drives = useDrives((s) => s.drives);

  // Unità: ogni 10 s e quando la finestra torna in primo piano (la guardia contro le letture
  // sovrapposte è in useDrives.refresh).
  useEffect(() => {
    const refresh = () => void useDrives.getState().refresh();
    refresh();
    const timer = setInterval(refresh, DRIVES_REFRESH_MS);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, []);

  const roots = drives.map((d) => ({
    path: d.root,
    label: [d.root.replace(/\\$/, ""), d.label].filter(Boolean).join(" "),
    detail: d.ready ? t("groups.free", { size: formatSize(d.free, i18n.language) }) : undefined,
  }));

  return (
    <>
      <div className="mc-side-h">{t("groups.drives")}</div>
      <FolderTree roots={roots} />
      <div className="mc-side-h">{t("shell.views.favorites")}</div>
      <FavoritesList edit={false} />
    </>
  );
}
