import { useTranslation } from "react-i18next";

import {
  currentLanguage,
  setLanguage,
  type SupportedLanguage,
} from "@/shared/i18n";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function LanguageSettingsCard() {
  const { t } = useTranslation();
  const language = currentLanguage();

  return (
    <SettingsOptionGroup>
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="font-medium text-foreground">{t("language.title")}</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("language.description")}
          </p>
        </div>
        <select
          aria-label={t("language.label")}
          className="min-w-48 rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) =>
            void setLanguage(event.currentTarget.value as SupportedLanguage)
          }
          value={language}
        >
          <option value="pt-BR">{t("language.portuguese")}</option>
          <option value="en-US">{t("language.english")}</option>
        </select>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
