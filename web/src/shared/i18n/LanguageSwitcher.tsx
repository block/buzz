import { Languages } from "lucide-react";
import { useTranslation } from "react-i18next";

import { currentLanguage, setLanguage, type SupportedLanguage } from "./index";

export function LanguageSwitcher() {
  const { t } = useTranslation();
  return (
    <label className="fixed right-4 top-4 z-50 flex items-center gap-2 rounded-full border border-black/10 bg-white/90 px-3 py-2 text-xs text-black shadow-sm backdrop-blur dark:border-white/10 dark:bg-black/80 dark:text-white">
      <Languages aria-hidden="true" className="h-4 w-4" />
      <span className="sr-only">{t("language.label")}</span>
      <select
        aria-label={t("language.label")}
        className="bg-transparent"
        onChange={(event) =>
          void setLanguage(event.currentTarget.value as SupportedLanguage)
        }
        value={currentLanguage()}
      >
        <option value="pt-BR">{t("language.portuguese")}</option>
        <option value="en-US">{t("language.english")}</option>
      </select>
    </label>
  );
}
