export type Lang = "en" | "zh";

const dict = {
  en: {
    brand: "Buzz Admin",
    nav_reports: "Reports",
    nav_feedback: "Feedback",
    loading: "Loading…",
    access_denied: "Access denied",
    load_failed: "Could not load data",
    retry: "Retry",
    lang: "Language",
  },
  zh: {
    brand: "Buzz 管理台",
    nav_reports: "举报",
    nav_feedback: "反馈",
    loading: "加载中…",
    access_denied: "无权限",
    load_failed: "无法加载数据",
    retry: "重试",
    lang: "语言",
  },
} as const;

export type MsgKey = keyof (typeof dict)["en"];

const STORAGE_KEY = "buzz-admin-lang";

export function getLang(): Lang {
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "zh" || v === "en" ? v : "zh";
}

export function setLang(lang: Lang) {
  localStorage.setItem(STORAGE_KEY, lang);
}

export function t(key: MsgKey, lang: Lang = getLang()): string {
  return dict[lang][key] ?? dict.en[key] ?? key;
}
