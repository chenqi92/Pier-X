import { createContext, useContext } from "react";
import type { Locale } from "../stores/useSettingsStore";
import en from "./en.json";
import zh from "./zh.json";

type Dictionary = Record<string, string>;
type TranslationVars = Record<string, string | number | null | undefined>;

const dictionaries: Record<Locale, Dictionary> = { en, zh };

export type I18nValue = {
  t: (key: string, vars?: TranslationVars) => string;
  locale: Locale;
};

export const I18nContext = createContext<I18nValue>({
  t: (key) => key,
  locale: "en",
});

function interpolate(template: string, vars?: TranslationVars) {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (_, key: string) => {
    const value = vars[key];
    return value === null || value === undefined ? "" : String(value);
  });
}

export function makeI18n(locale: Locale): I18nValue {
  const dict = dictionaries[locale] ?? {};
  return {
    t: (key: string, vars?: TranslationVars) => interpolate(dict[key] || key, vars),
    locale,
  };
}

export function useI18n() {
  return useContext(I18nContext);
}
