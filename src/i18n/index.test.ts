import { expect, test } from "vitest";
import en from "./en.json";
import it from "./it.json";
import { errorKey } from ".";

test("UNSUPPORTED with a known detail has its own message in both languages", () => {
  for (const detail of ["encrypted", "sparse", "reparse_point", "too_many_entries"]) {
    expect(errorKey({ code: "UNSUPPORTED", detail })).toEqual([`errors.UNSUPPORTED_${detail}`, "errors.UNSUPPORTED"]);
    const key = `UNSUPPORTED_${detail}` as keyof typeof en.errors;
    expect(en.errors[key]).toBeTruthy();
    expect(it.errors[key as keyof typeof it.errors]).toBeTruthy();
  }
  // Senza dettaglio, o con il dettaglio di sistema degli altri codici, il messaggio del codice.
  expect(errorKey({ code: "UNSUPPORTED", detail: null })).toBe("errors.UNSUPPORTED");
  expect(errorKey({ code: "IO", detail: "Accesso negato." })).toBe("errors.IO");
});
