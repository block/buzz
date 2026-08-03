import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  useEffect,
} from "react";

import {
  DEFAULT_LANG,
  type Lang,
  type MsgKey,
  loadStoredLang,
  persistLang,
  translate,
} from "./messages";

type I18nContextValue = {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: MsgKey, vars?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18nContextValue | undefined>(undefined);

function applyDocumentLang(lang: Lang) {
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() =>
    loadStoredLang(window.localStorage),
  );

  useEffect(() => {
    applyDocumentLang(lang);
  }, [lang]);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    persistLang(window.localStorage, next);
    applyDocumentLang(next);
  }, []);

  const t = useCallback(
    (key: MsgKey, vars?: Record<string, string | number>) =>
      translate(key, lang, vars),
    [lang],
  );

  const value = useMemo(
    () => ({
      lang,
      setLang,
      t,
    }),
    [lang, setLang, t],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    throw new Error("useI18n must be used within I18nProvider");
  }
  return ctx;
}

/** Safe for optional trees (tests / isolated previews). Falls back to default zh. */
export function useOptionalI18n(): I18nContextValue {
  const ctx = useContext(I18nContext);
  if (ctx) {
    return ctx;
  }
  return {
    lang: DEFAULT_LANG,
    setLang: () => {},
    t: (key, vars) => translate(key, DEFAULT_LANG, vars),
  };
}
