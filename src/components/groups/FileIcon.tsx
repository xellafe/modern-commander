import {
  CornerLeftUp, Disc, File, FileArchive, FileBraces, FileCode, FileImage, FileMusic, FileSpreadsheet,
  FileSymlink, FileTerminal, FileText, FileVideoCamera, Folder, FolderSymlink, HardDrive, Network, Usb,
  type LucideIcon,
} from "lucide-react";
import type { DriveKind } from "../../ipc/types";
import { fileKind, type FileKind } from "../../lib/format";
import type { Row } from "../../state/tabView";

const BY_KIND: Record<FileKind, LucideIcon> = {
  folder: Folder, image: FileImage, code: FileCode, data: FileBraces, doc: FileText, pdf: FileText,
  archive: FileArchive, audio: FileMusic, video: FileVideoCamera, sheet: FileSpreadsheet,
  exec: FileTerminal, file: File,
};
const BY_DRIVE: Record<DriveKind, LucideIcon> = {
  fixed: HardDrive, removable: Usb, network: Network, cdrom: Disc, other: HardDrive,
};

// Il colore viene da --mc-icon-<tipo>: monocromatico nel tema scuro, tenue nel chiaro.
export function FileIcon({ row, size = 14 }: { row: Row; size?: number }) {
  const kind = row.drive || row.isParent ? "folder" : fileKind(row.name, row.isDir);
  const Icon = row.isParent
    ? CornerLeftUp
    : row.drive
      ? BY_DRIVE[row.drive.kind]
      : row.isSymlink
        ? (row.isDir ? FolderSymlink : FileSymlink)
        : BY_KIND[kind];
  return <Icon className="mc-icon" size={size} strokeWidth={1.6} style={{ color: `var(--mc-icon-${kind})` }} aria-hidden />;
}
