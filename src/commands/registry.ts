export interface Command {
  id: string;
  titleKey: string;
  when: 'fileList' | 'global';
  /** true = l'auto-repeat del tasto riesegue il comando (spostamenti, marcatura); default: solo la prima pressione */
  repeat?: boolean;
  run(): void | Promise<void>;
}

export function shouldExecute(
  cmd: Command | undefined,
  opts: { repeat: boolean; focusInText: boolean },
): boolean {
  if (!cmd) return false;
  if (opts.focusInText && cmd.when !== 'global') return false;
  return !opts.repeat || cmd.repeat === true;
}

const commands = new Map<string, Command>();

export const registerCommand = (c: Command): void => {
  commands.set(c.id, c);
};

export const getCommand = (id: string): Command | undefined => commands.get(id);

export const getCommands = (): Command[] => [...commands.values()];

// Un id non registrato (comando di un'altra fase) è un no-op silenzioso.
export async function executeCommand(id: string): Promise<void> {
  await commands.get(id)?.run();
}
