import { expect, test } from "vitest";
import { buildKeymap } from "../commands/keymap";
import { DEFAULT_KEYMAP } from "../commands/tcKeymap";
import en from "../i18n/en.json";
import it from "../i18n/it.json";
import { fkeyLabels } from "./fkeys";

const ALL = new Set([
  "cm_List", "cm_Edit", "cm_Copy", "cm_CopySamepanel", "cm_RenMov", "cm_RenameOnly",
  "cm_MkDir", "cm_Delete", "mc_DeletePermanent", "cm_Exit", "cm_PackFiles", "cm_UnpackFiles", "cm_Properties",
  "cm_SrcShort", "cm_SrcLong", "cm_SrcByName", "cm_SrcByExt", "cm_SrcByDateTime", "cm_SrcBySize", "cm_SrcUnsorted",
  "cm_SrcOpenDrives", "cm_SrcThumbs",
]);
const DEFAULTS = buildKeymap(DEFAULT_KEYMAP, []);
const titles = (registered: Set<string>) => (id: string) => (registered.has(id) ? `commands.${id}` : undefined);
const none = { shift: false, ctrl: false, alt: false };
const pairs = (m: Parameters<typeof fkeyLabels>[0], a = ALL, keymap = DEFAULTS) =>
  fkeyLabels(m, keymap, titles(a)).map((s) => [s.key, s.commandId]);

test("senza modificatori: livello base con Alt+F4", () => {
  expect(pairs(none)).toEqual([
    ["F2", "cm_RenameOnly"], ["F3", "cm_List"], ["F4", "cm_Edit"], ["F5", "cm_Copy"], ["F6", "cm_RenMov"],
    ["F7", "cm_MkDir"], ["F8", "cm_Delete"], ["Alt+F4", "cm_Exit"],
  ]);
});

test("con Shift: solo il livello Shift", () => {
  expect(pairs({ ...none, shift: true })).toEqual([
    ["F5", "cm_CopySamepanel"], ["F6", "cm_RenameOnly"], ["F8", "mc_DeletePermanent"],
  ]);
});

test("con Alt: unità, uscita, comprimi ed estrai", () => {
  expect(pairs({ ...none, alt: true })).toEqual([
    ["F1", "cm_SrcOpenDrives"], ["F4", "cm_Exit"], ["F5", "cm_PackFiles"], ["F9", "cm_UnpackFiles"],
  ]);
});

test("con Ctrl+Shift: miniature", () => {
  expect(pairs({ shift: true, ctrl: true, alt: false })).toEqual([["F1", "cm_SrcThumbs"]]);
});

test("con Ctrl: vista e ordinamento come in TC", () => {
  expect(pairs({ ...none, ctrl: true })).toEqual([
    ["F1", "cm_SrcShort"], ["F2", "cm_SrcLong"], ["F3", "cm_SrcByName"], ["F4", "cm_SrcByExt"],
    ["F5", "cm_SrcByDateTime"], ["F6", "cm_SrcBySize"], ["F7", "cm_SrcUnsorted"],
  ]);
});

test("combinazione senza livello (Ctrl+Alt, Alt+Shift): barra vuota", () => {
  expect(pairs({ shift: false, ctrl: true, alt: true })).toEqual([]);
  expect(pairs({ shift: true, ctrl: false, alt: true })).toEqual([]);
});

test("un comando non registrato non compare", () => {
  const without = new Set(["cm_Copy", "cm_Delete"]);
  expect(pairs(none, without)).toEqual([["F5", "cm_Copy"], ["F8", "cm_Delete"]]);
  expect(pairs({ ...none, shift: true }, without)).toEqual([]);
});

test("ogni etichetta dei cinque livelli esiste in en.json e it.json", () => {
  const all = [none, { ...none, shift: true }, { ...none, alt: true }, { ...none, ctrl: true }, { ...none, ctrl: true, shift: true }]
    .flatMap((m) => fkeyLabels(m, DEFAULTS, titles(ALL)));
  expect(all.length).toBe(8 + 3 + 4 + 7 + 1);
  const lookup = (res: object, key: string) => key.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown>)?.[k], res);
  for (const { labelKey } of all) {
    expect(typeof lookup(en, labelKey), `en ${labelKey}`).toBe("string");
    expect(typeof lookup(it, labelKey), `it ${labelKey}`).toBe("string");
  }
});

test("la barra segue la mappa dell'utente: tasti tolti, spostati e comandi senza etichetta breve", () => {
  const keymap = buildKeymap(DEFAULT_KEYMAP, [
    { key: "Ctrl+F1", command: "-cm_SrcShort" },
    { key: "Ctrl+Shift+F1", command: "-cm_SrcThumbs" },
    { key: "Ctrl+Shift+F2", command: "cm_SrcThumbs" },
    { key: "F9", command: "cm_Properties" },
  ]);
  expect(pairs({ ...none, ctrl: true }, ALL, keymap)[0]).toEqual(["F2", "cm_SrcLong"]);
  expect(pairs({ shift: true, ctrl: true, alt: false }, ALL, keymap)).toEqual([["F2", "cm_SrcThumbs"]]);
  const f9 = fkeyLabels(none, keymap, titles(ALL)).find((s) => s.key === "F9");
  expect(f9).toEqual({ key: "F9", commandId: "cm_Properties", labelKey: "commands.cm_Properties" });
});
