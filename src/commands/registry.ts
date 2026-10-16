export interface Command {
  id: string;
  titleKey: string;
  when: 'fileList' | 'global';
  run(): void | Promise<void>;
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
