import { ChevronDown, ChevronRight, Folder, HardDrive } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ensureDirTab } from "../../commands/navigation";
import { joinPath } from "../../lib/paths";
import { childDirs, useFolderTree } from "../../state/folderTree";
import { useLayout } from "../../state/layout";
import { useSettings } from "../../state/settings";
import { useTabViews } from "../../state/tabViews";

export interface TreeRoot {
  path: string;
  label: string;
  /** Testo attenuato a destra (es. spazio libero dell'unità). */
  detail?: string;
}

const samePath = (a: string, b: string) => a.replace(/\\+$/, "").toLowerCase() === b.replace(/\\+$/, "").toLowerCase();

/** Un click apre la cartella nella scheda cartella attiva del gruppo attivo (se la scheda attiva è un file, torna a chi l'ha aperta). */
export function openInActiveTab(path: string) {
  void useTabViews.getState().load(ensureDirTab(path), path);
}

// Albero a caricamento pigro: le sottocartelle si leggono alla prima espansione.
export function FolderTree({ roots }: { roots: TreeRoot[] }) {
  const activePath = useLayout((s) => s.tabs[s.groups.find((g) => g.id === s.activeGroupId)!.activeTabId].path);
  return (
    <div role="tree">
      {roots.map((r) => (
        <TreeNode key={r.path} path={r.path} label={r.label} detail={r.detail} depth={0} activePath={activePath} />
      ))}
    </div>
  );
}

interface NodeProps {
  path: string;
  label: string;
  detail?: string;
  depth: number;
  activePath: string;
}

function TreeNode({ path, label, detail, depth, activePath }: NodeProps) {
  const { t } = useTranslation();
  const expanded = useFolderTree((s) => !!s.expanded[path]);
  const node = useFolderTree((s) => s.nodes[path]);
  const showHidden = useSettings((s) => s.showHidden);
  const showSystem = useSettings((s) => s.showSystem);
  const names = useMemo(
    () => (node?.status === "ready" || (node?.status === "loading" && node.dirs.length) ? childDirs(node.dirs, showHidden, showSystem) : []),
    [node, showHidden, showSystem],
  );
  const failed = node?.status === "error";
  const leaf = failed || (node?.status === "ready" && names.length === 0); // niente da espandere
  const active = samePath(path, activePath);

  return (
    <div role="treeitem" aria-level={depth + 1} aria-expanded={leaf ? undefined : expanded} aria-selected={active}>
      <div
        className={`mc-tree-row${active ? " on" : ""}${failed ? " err" : ""}`}
        style={{ paddingLeft: 6 + depth * 12 }}
        title={failed ? t(`errors.${node.error}`) : path}
        onClick={() => {
          useFolderTree.getState().retry(path); // un click su un nodo in errore lo riprova alla prossima espansione
          openInActiveTab(path);
        }}
      >
        <span
          className="mc-tree-chev"
          onClick={(e) => {
            e.stopPropagation();
            if (!leaf) void useFolderTree.getState().toggle(path);
          }}
        >
          {leaf ? null : expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
        {depth === 0 ? <HardDrive size={14} className="mc-tree-icon" /> : <Folder size={14} className="mc-tree-icon" />}
        <span className="mc-tree-name">{label}</span>
        {detail && <span className="mc-tree-detail">{detail}</span>}
      </div>
      {expanded && names.length > 0 && (
        <div role="group">
          {names.map((n) => (
            <TreeNode key={n} path={joinPath(path, n)} label={n} depth={depth + 1} activePath={activePath} />
          ))}
        </div>
      )}
    </div>
  );
}
