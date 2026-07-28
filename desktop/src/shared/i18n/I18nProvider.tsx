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
  LANG_STORAGE_KEY,
  type Lang,
  type MsgKey,
  isLang,
  translate,
} from "./messages";

type I18nContextValue = {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: MsgKey, vars?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18nContextValue | undefined>(undefined);

function readStoredLang(): Lang {
  try {
    const stored = window.localStorage.getItem(LANG_STORAGE_KEY);
    if (isLang(stored)) {
      return stored;
    }
  } catch {
    // ignore quota / private mode
  }
  return DEFAULT_LANG;
}

function applyDocumentLang(lang: Lang) {
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() => readStoredLang());

  useEffect(() => {
    applyDocumentLang(lang);
  }, [lang]);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      window.localStorage.setItem(LANG_STORAGE_KEY, next);
    } catch {
      // ignore
    }
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
