import { createContext, useContext } from "react";
import type { Locale } from "../stores/useSettingsStore";
import en from "./en.json";
import zh from "./zh.json";
import { zhExtra } from "./zhExtra";

type Dictionary = Record<string, string>;
type TranslationVars = Record<string, string | number | null | undefined>;

const dictionaries: Record<Locale, Dictionary> = {
  en,
  zh: { ...zh, ...zhExtra },
};

export type I18nValue = {
  t: (key: string, vars?: TranslationVars) => string;
  locale: Locale;
};

export const I18nContext = createContext<I18nValue>({
  t: (key) => key,
  locale: "zh",
});

function interpolate(template: string, vars?: TranslationVars) {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (_, key: string) => {
    const value = vars[key];
    return value === null || value === undefined ? "" : String(value);
  });
}

export function translate(locale: Locale, key: string, vars?: TranslationVars) {
  const dict = dictionaries[locale] ?? {};
  return interpolate(dict[key] || key, vars);
}

export function makeI18n(locale: Locale): I18nValue {
  return {
    t: (key: string, vars?: TranslationVars) => translate(locale, key, vars),
    locale,
  };
}

export function useI18n() {
  return useContext(I18nContext);
}
