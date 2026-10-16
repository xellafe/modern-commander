import { expect, test } from "vitest";
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

test("ogni slot porta una chiave i18n", () => {
  expect(fkeyLabels(none, ALL).every((s) => s.labelKey.startsWith("shell.fkeys."))).toBe(true);
});
