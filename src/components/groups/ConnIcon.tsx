import { Globe, Server } from "lucide-react";
import type { Scheme } from "../../ipc/types";

/** Icona di una connessione remota: `Globe` per HTTP(S), `Server` per gli altri schemi. */
export const ConnIcon = ({ scheme, size = 12 }: { scheme: Scheme; size?: number }) =>
  scheme.startsWith("http") ? <Globe size={size} aria-hidden /> : <Server size={size} aria-hidden />;
