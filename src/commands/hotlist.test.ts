import { expect, test } from "vitest";
import en from "../i18n/en.json";
import it from "../i18n/it.json";
import { registerHotlistCommands } from "./hotlist";
import { getCommand } from "./registry";
import { DEFAULT_KEYMAP } from "./tcKeymap";

test("cm_DirectoryHotlist is registered, bound to Ctrl+D, and titled in both languages", () => {
  registerHotlistCommands();
  expect(getCommand("cm_DirectoryHotlist")).toMatchObject({ titleKey: "commands.cm_DirectoryHotlist", when: "fileList" });
  expect(DEFAULT_KEYMAP["Ctrl+D"]).toBe("cm_DirectoryHotlist");
  expect(en.commands.cm_DirectoryHotlist).toBeTruthy();
  expect(it.commands.cm_DirectoryHotlist).toBeTruthy();
});
