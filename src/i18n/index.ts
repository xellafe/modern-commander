import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import it from "./it.json";

export const initI18n = (language: "en" | "it") =>
  i18n.use(initReactI18next).init({
    lng: language,
    fallbackLng: "en",
    resources: { en: { translation: en }, it: { translation: it } },
    interpolation: { escapeValue: false },
  });

/**
 * Chiave del messaggio di un errore: per `UNSUPPORTED` con un dettaglio noto (`encrypted`, `sparse`, `reparse_point`,
 * `too_many_entries`) la sua, con quella del codice come riserva; il `detail` degli altri codici è testo di sistema.
 */
export const errorKey = (e: { code: string; detail?: string | null }): string | string[] =>
  e.code === "UNSUPPORTED" && e.detail ? [`errors.UNSUPPORTED_${e.detail}`, "errors.UNSUPPORTED"] : `errors.${e.code}`;

export default i18n;
