import { FileArchive, Globe, Server } from "lucide-react";
import type { Scheme } from "../../ipc/types";

/** Icona di una connessione remota: `Globe` per HTTP(S), `Server` per gli altri schemi. */
export const ConnIcon = ({ scheme, size = 12 }: { scheme: Scheme; size?: number }) =>
  scheme.startsWith("http") ? <Globe size={size} aria-hidden /> : <Server size={size} aria-hidden />;

/** Icona di un archivio aperto come cartella (scheda e crumb dell'archivio). */
export const ArchiveIcon = ({ size = 12 }: { size?: number }) => <FileArchive size={size} aria-hidden />;
