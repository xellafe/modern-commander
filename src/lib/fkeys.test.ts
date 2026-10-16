import { expect, test } from "vitest";
import en from "../i18n/en.json";
import it from "../i18n/it.json";
import { fkeyLabels } from "./fkeys";

const ALL = new Set([
  "cm_List", "cm_Edit", "cm_Copy", "cm_CopySamepanel", "cm_RenMov", "cm_RenameOnly",
  "cm_MkDir", "cm_Delete", "mc_DeletePermanent", "cm_Exit", "cm_PackFiles",
]);
const none = { shift: false, ctrl: false, alt: false };
const pairs = (m: Parameters<typeof fkeyLabels>[0], a = ALL) => fkeyLabels(m, a).map((s) => [s.key, s.commandId]);

test("senza modificatori: livello base con Alt+F4", () => {
  expect(pairs(none)).toEqual([
    ["F3", "cm_List"], ["F4", "cm_Edit"], ["F5", "cm_Copy"], ["F6", "cm_RenMov"],
    ["F7", "cm_MkDir"], ["F8", "cm_Delete"], ["Alt+F4", "cm_Exit"],
  ]);
});

test("con Shift: solo il livello Shift", () => {
  expect(pairs({ ...none, shift: true })).toEqual([
    ["F5", "cm_CopySamepanel"], ["F6", "cm_RenameOnly"], ["F8", "mc_DeletePermanent"],
  ]);
});

test("con Alt: solo Alt+F4", () => {
  expect(pairs({ ...none, alt: true })).toEqual([["F4", "cm_Exit"]]);
});

test("combinazione senza livello (Ctrl, Shift+Ctrl): barra vuota", () => {
  expect(pairs({ ...none, ctrl: true })).toEqual([]);
  expect(pairs({ shift: true, ctrl: true, alt: false })).toEqual([]);
});

test("un comando non registrato non compare", () => {
  const without = new Set(["cm_Copy", "cm_Delete"]);
  expect(pairs(none, without)).toEqual([["F5", "cm_Copy"], ["F8", "cm_Delete"]]);
  expect(fkeyLabels({ ...none, shift: true }, without)).toEqual([]);
});

test("ogni etichetta dei tre livelli esiste in en.json e it.json", () => {
  const all = [none, { ...none, shift: true }, { ...none, alt: true }].flatMap((m) => fkeyLabels(m, ALL));
  expect(all.length).toBe(7 + 3 + 1);
  const lookup = (res: object, key: string) => key.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown>)?.[k], res);
  for (const { labelKey } of all) {
    expect(typeof lookup(en, labelKey), `en ${labelKey}`).toBe("string");
    expect(typeof lookup(it, labelKey), `it ${labelKey}`).toBe("string");
  }
});
