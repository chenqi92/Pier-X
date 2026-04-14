import { createContext, useContext } from "react";
import type { Locale } from "../stores/useSettingsStore";
import en from "./en.json";
import zh from "./zh.json";

type Dictionary = Record<string, string>;

const dictionaries: Record<Locale, Dictionary> = { en, zh };

export type I18nValue = {
  t: (key: string) => string;
  locale: Locale;
};

export const I18nContext = createContext<I18nValue>({
  t: (key) => key,
  locale: "en",
});

export function makeI18n(locale: Locale): I18nValue {
  const dict = dictionaries[locale] ?? {};
  return {
    t: (key: string) => dict[key] || key,
    locale,
  };
}

export function useI18n() {
  return useContext(I18nContext);
}
