import { activeTab, useLayout } from '../state/layout';

/**
 * Dove vale il comando: 'fileList' = lista di una scheda cartella con il focus fuori dai campi;
 * 'group' = qualunque scheda (anche viewer/editor), focus fuori dai campi; 'global' = sempre.
 */
export interface Command {
  id: string;
  titleKey: string;
  when: 'fileList' | 'group' | 'global';
  /** true = l'auto-repeat del tasto riesegue il comando (spostamenti, marcatura); default: solo la prima pressione */
  repeat?: boolean;
  run(): void | Promise<void>;
}

export function shouldExecute(
  cmd: Command | undefined,
  opts: { repeat: boolean; focusInText: boolean; dirTab: boolean },
): boolean {
  if (!cmd) return false;
  if (opts.focusInText && cmd.when !== 'global') return false;
  if (cmd.when === 'fileList' && !opts.dirTab) return false;
  return !opts.repeat || cmd.repeat === true;
}

const commands = new Map<string, Command>();

export const registerCommand = (c: Command): void => {
  commands.set(c.id, c);
};

export const getCommand = (id: string): Command | undefined => commands.get(id);

export const getCommands = (): Command[] => [...commands.values()];

/** Un comando 'fileList' non gira se la scheda attiva è un file: vale anche per la barra F-key e per la palette. */
export const isRunnable = (cmd: Command): boolean =>
  cmd.when !== 'fileList' || !activeTab(useLayout.getState()).file;

// Un id non registrato (comando di un'altra fase) è un no-op silenzioso.
export async function executeCommand(id: string): Promise<void> {
  const cmd = commands.get(id);
  if (cmd && isRunnable(cmd)) await cmd.run();
}
