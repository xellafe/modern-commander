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

export default i18n;
