// Idiomas de la interfaz. El español es el diccionario de referencia: los
// demás tienen su mismo tipo, así que TypeScript avisa si falta un texto.
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { setFormatLocale } from "../lib/format";
import type { Lang } from "../lib/types";
import { setCurrent } from "./current";
import { de } from "./de";
import { en } from "./en";
import { es, type Dict } from "./es";
import { fr } from "./fr";
import { pt } from "./pt";

export type { Dict };

export const LANGS: Lang[] = ["es", "en", "pt", "fr", "de"];

/** Cada idioma con su propio nombre, para el selector. */
export const LANG_NAMES: Record<Lang, string> = {
  es: "Español",
  en: "English",
  pt: "Português",
  fr: "Français",
  de: "Deutsch",
};

const DICTS: Record<Lang, Dict> = { es, en, pt, fr, de };
const LOCALES: Record<Lang, string> = { es: "es-ES", en: "en-US", pt: "pt-BR", fr: "fr-FR", de: "de-DE" };

/** Idioma del navegador mientras llega el que decide la app (el elegido o el del sistema). */
function guess(): Lang {
  for (const tag of navigator.languages ?? [navigator.language]) {
    const code = tag.slice(0, 2).toLowerCase() as Lang;
    if (LANGS.includes(code)) return code;
  }
  return "en";
}

function activate(lang: Lang): Dict {
  setFormatLocale(LOCALES[lang]);
  setCurrent(DICTS[lang]);
  return DICTS[lang];
}

interface I18n {
  lang: Lang;
  t: Dict;
  setLang: (lang: Lang) => void;
}

const I18nContext = createContext<I18n>({ lang: "es", t: es, setLang: () => {} });

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLang] = useState<Lang>(guess);
  const t = useMemo(() => activate(lang), [lang]);
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);
  const value = useMemo(() => ({ lang, t, setLang }), [lang, t]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export const useT = () => useContext(I18nContext).t;

export const useI18n = () => useContext(I18nContext);
