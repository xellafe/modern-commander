// Sessioni del terminale: elenco in `useTerminals`, istanze xterm in una mappa fuori dallo store (sono oggetti DOM).
import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { create } from "zustand";
import { chordFromEvent, type KeyLike } from "../commands/keys";
import { getCommand } from "../commands/registry";
import { onTermExit, openPath, termKill, termResize, termShells, termSpawn, termWrite } from "../ipc/api";
import type { ShellKind } from "../ipc/types";
import { folderName } from "../lib/paths";
import { loadXterm, termFont, termTheme, TERM_FONT_SIZE } from "../lib/xterm";
import { useKeybindings } from "./keybindings";
import { useSettings } from "./settings";
import { useUi } from "./ui";

/** `id` = id del PTY; `title` = nome della cartella in cui è partita (la shell si mostra col suo nome tradotto). */
export interface TermSession { id: number; title: string; shell: ShellKind }

type PanelTab = "operations" | "terminal";

/** Toglie `id`: se era l'attiva diventa attiva la vicina a destra, altrimenti quella a sinistra (`null` se non resta nulla). */
export function removeSession(sessions: TermSession[], activeId: number | null, id: number): { sessions: TermSession[]; activeId: number | null } {
  const i = sessions.findIndex((s) => s.id === id);
  if (i < 0) return { sessions, activeId };
  const rest = sessions.filter((s) => s.id !== id);
  if (activeId !== id) return { sessions: rest, activeId };
  return { sessions: rest, activeId: (rest[i] ?? rest[i - 1])?.id ?? null };
}

/**
 * Ctrl+ò: nasconde il pannello se mostra già il terminale, altrimenti lo mostra su quella scheda (`create`: nessuna
 * sessione ancora). `pending` = sessioni il cui avvio non è finito: contano, o un secondo Ctrl+ò ne aprirebbe un'altra.
 */
export function toggleTerminal(panel: { visible: boolean; tab: PanelTab }, hasSessions: boolean, pending = 0): { visible: boolean; tab: PanelTab; create: boolean } {
  if (panel.visible && panel.tab === "terminal") return { visible: false, tab: "terminal", create: false };
  return { visible: true, tab: "terminal", create: !hasSessions && pending === 0 };
}

/** Ctrl+V, Ctrl+Shift+V e Shift+Insert: l'incolla lo fa il browser con l'evento `paste` sulla textarea di xterm. */
const isPasteChord = (e: KeyLike) => (e.ctrlKey && !e.altKey && e.keyCode === 86) || (e.shiftKey && !e.ctrlKey && !e.altKey && e.keyCode === 45);

/**
 * Per `attachCustomKeyEventHandler`: false = xterm ignora il tasto e l'evento prosegue (senza `preventDefault`).
 * Succede per i chord legati a un comando `global` (Ctrl+ò, Ctrl+Shift+P…), che sale a `window` dove `useKeyboard` lo
 * esegue, e per l'incolla, che resta al browser (altrimenti xterm manderebbe `^V` e annullerebbe il tasto: `paste` non
 * scatterebbe mai). Tutto il resto (Ctrl+C, Tab, frecce) è della shell.
 */
export function passToShell(e: KeyLike, keymap: Map<string, string>, isGlobal: (commandId: string) => boolean): boolean {
  if (isPasteChord(e)) return false;
  const id = keymap.get(chordFromEvent(e));
  return id === undefined || !isGlobal(id);
}

/** Sessioni il cui avvio non è ancora finito (vedi `toggleTerminal`). */
export const pendingTerminals = () => pending.size;

/** La shell di un nuovo terminale: quella scelta se è avviabile, altrimenti la migliore (pwsh, powershell, cmd: come `default_shell` del backend). */
export function resolveShell(wanted: "auto" | ShellKind, available: ShellKind[]): ShellKind {
  if (wanted !== "auto" && available.includes(wanted)) return wanted;
  return (["pwsh", "powershell"] as const).find((k) => available.includes(k)) ?? "cmd";
}

interface Size { cols: number; rows: number }

/**
 * Collega xterm al PTY prima che `termSpawn` restituisca l'id: scritture e ridimensionamenti prima dell'id restano in
 * coda e partono alla risoluzione, nell'ordine. Senza, la risposta di xterm alla richiesta di posizione del cursore
 * (`ESC[6n`) andrebbe persa e la shell non partirebbe mai. `spawned` = dimensioni passate a `termSpawn`.
 */
export function createLink(io: { write(id: number, data: string): void; resize(id: number, cols: number, rows: number): void }, spawned: Size) {
  let id: number | null = null;
  const queue: string[] = [];
  let latest = spawned;
  return {
    input(data: string) {
      if (id === null) queue.push(data);
      else io.write(id, data);
    },
    resize(cols: number, rows: number) {
      latest = { cols, rows };
      if (id !== null) io.resize(id, cols, rows);
    },
    resolve(newId: number) {
      id = newId;
      for (const data of queue.splice(0)) io.write(newId, data);
      // un ridimensionamento nel frattempo: il PTY è nato con `spawned`
      if (latest.cols !== spawned.cols || latest.rows !== spawned.rows) io.resize(newId, latest.cols, latest.rows);
    },
  };
}

/** `term-exit` può arrivare prima che l'id sia noto: si ricorda e lo si gestisce alla `register`. */
export function createExits(isKnown: (id: number) => boolean, onExit: (id: number) => void) {
  const early = new Set<number>();
  return {
    exit(id: number) {
      if (isKnown(id)) onExit(id);
      else early.add(id);
    },
    register(id: number) {
      if (early.delete(id)) onExit(id);
    },
  };
}

interface Instance {
  term: Terminal;
  fit: FitAddon;
  /** Contenitore proprio dell'istanza: lo monta `layoutTerminals` nell'area del pannello. */
  el: HTMLElement;
  opened: boolean;
  disposed: boolean;
  id: number | null;
}

/** Dopo `term-exit` gli ultimi blocchi di output possono ancora arrivare: l'istanza si distrugge dopo questa pausa. */
const EXIT_GRACE_MS = 1000;

const instances = new Map<number, Instance>();
/** Istanze il cui `termSpawn` non è ancora tornato (id ignoto). */
const pending = new Set<Instance>();
let host: HTMLElement | null = null;

const all = () => [...pending, ...instances.values()];

const exits = createExits((id) => instances.has(id), handleExit);
let listener: Promise<unknown> | undefined;
const listening = () =>
  (listener ??= onTermExit((e) => exits.exit(e.id)).catch((err) => {
    listener = undefined;
    throw err;
  }));

function disposeInstance(inst: Instance) {
  inst.disposed = true;
  inst.el.remove();
  inst.term.dispose(); // porta con sé l'addon
}

/** Toglie la sessione dall'elenco; se era l'ultima, con il terminale in vista, il pannello si nasconde. Falso se non c'era. */
function drop(id: number): boolean {
  const { sessions, activeId } = useTerminals.getState();
  if (!sessions.some((s) => s.id === id)) return false;
  const next = removeSession(sessions, activeId, id);
  useTerminals.setState(next);
  if (!next.sessions.length && useUi.getState().bottomPanelTab === "terminal") useUi.setState({ bottomPanelVisible: false });
  return true;
}

function handleExit(id: number) {
  const inst = instances.get(id);
  if (!inst) return;
  instances.delete(id);
  drop(id);
  inst.el.classList.remove("on");
  setTimeout(() => disposeInstance(inst), EXIT_GRACE_MS);
}

/** L'area del pannello che ospita le istanze (la registra `TerminalView`). */
export function setTerminalHost(el: HTMLElement | null) {
  host = el;
  layoutTerminals();
}

/**
 * Monta le istanze nell'area, apre quelle ancora chiuse e adatta tutte alla sua dimensione (`onResize` avvisa il PTY).
 * xterm si apre solo con l'area visibile: con il pannello nascosto non fa nulla, e riparte al primo `ResizeObserver`.
 */
export function layoutTerminals() {
  if (!host || !host.clientWidth || !host.clientHeight) return;
  const { activeId } = useTerminals.getState();
  for (const inst of all()) {
    if (!inst.el.isConnected) host.appendChild(inst.el);
    const justOpened = !inst.opened;
    if (justOpened) {
      inst.term.open(inst.el);
      inst.opened = true;
    }
    inst.fit.fit();
    if (justOpened && inst.id === activeId) inst.term.focus();
  }
}

/** Mostra solo l'istanza attiva (le altre restano montate, nascoste) e, con `focus`, le dà il focus. */
export function showActiveTerminal(focus: boolean) {
  const { activeId } = useTerminals.getState();
  for (const [id, inst] of instances) inst.el.classList.toggle("on", id === activeId);
  if (focus && activeId !== null && instances.get(activeId)?.opened) instances.get(activeId)!.term.focus();
}

/** Ricolora tutte le istanze dai token del tema corrente. */
export function applyTerminalTheme() {
  for (const inst of all()) inst.term.options.theme = termTheme();
}

interface TerminalsState {
  sessions: TermSession[];
  activeId: number | null;
  /** Apre un terminale in `cwd` (vuota = cartella utente); `shell` assente = l'impostazione `terminalShell`. */
  create(cwd: string, shell?: ShellKind): Promise<void>;
  /** Chiude la scheda e ferma la shell; l'istanza xterm si distrugge al `term-exit` che ne segue. */
  close(id: number): void;
  activate(id: number): void;
}

const io = {
  write: (id: number, data: string) => termWrite(id, data).catch(console.error),
  resize: (id: number, cols: number, rows: number) => termResize(id, cols, rows).catch(console.error),
};

export const useTerminals = create<TerminalsState>((set, get) => ({
  sessions: [],
  activeId: null,

  create: async (cwd, shell) => {
    let inst: Instance | undefined;
    try {
      const [{ Terminal, FitAddon }, available] = await Promise.all([loadXterm(), termShells(), listening()]);
      const kind = resolveShell(shell ?? useSettings.getState().terminalShell, available);
      const term = new Terminal({
        fontFamily: termFont(),
        fontSize: TERM_FONT_SIZE,
        cursorBlink: true,
        scrollback: 5000,
        theme: termTheme(),
        windowsPty: { backend: "conpty" },
        // L'output non è fidato: i link OSC 8 si aprono solo se http(s), fuori dall'app (il predefinito usa `window.open`).
        linkHandler: { activate: (_e, uri) => void (/^https?:/i.test(uri) && openPath(uri).catch(() => {})) },
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      const el = document.createElement("div");
      el.className = "mc-term";
      const created: Instance = { term, fit, el, opened: false, disposed: false, id: null };
      inst = created;
      pending.add(created);
      layoutTerminals(); // con l'area visibile apre e adatta subito: dimensioni giuste già alla partenza

      const spawned = { cols: term.cols, rows: term.rows };
      const link = createLink(io, spawned);
      // Prima di `termSpawn`: la shell parte solo dopo che xterm ha risposto alla richiesta di posizione del cursore.
      term.onData(link.input);
      term.onResize(({ cols, rows }) => link.resize(cols, rows));
      term.attachCustomKeyEventHandler((e) => {
        if (e.type !== "keydown") return true;
        // Ctrl+C con una selezione copia (la selezione si toglie), senza inviare ^C
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.keyCode === 67 && term.hasSelection()) {
          void navigator.clipboard.writeText(term.getSelection()).catch(console.error);
          term.clearSelection();
          e.preventDefault();
          return false;
        }
        return passToShell(e, useKeybindings.getState().keymap, (id) => getCommand(id)?.when === "global");
      });

      const id = await termSpawn(kind, cwd, spawned.cols, spawned.rows, (bytes) => {
        if (!created.disposed) term.write(bytes);
      });
      created.id = id;
      pending.delete(created);
      instances.set(id, created);
      link.resolve(id);
      set((s) => ({ sessions: [...s.sessions, { id, title: cwd ? folderName(cwd) : "", shell: kind }], activeId: id }));
      exits.register(id); // un `term-exit` già arrivato toglie subito la sessione
    } catch (e) {
      if (inst) {
        pending.delete(inst);
        disposeInstance(inst);
      }
      if (!get().sessions.length && useUi.getState().bottomPanelTab === "terminal") useUi.setState({ bottomPanelVisible: false });
      throw e;
    }
  },

  close: (id) => {
    drop(id);
    instances.get(id)?.el.classList.remove("on");
    termKill(id).catch(console.error);
  },

  activate: (id) => {
    if (get().sessions.some((s) => s.id === id)) set({ activeId: id });
  },
}));
