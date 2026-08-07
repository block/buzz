import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  isAppLanguagePreference,
  readStoredLanguagePreference,
  setAppLanguage,
} from "@/i18n";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function LanguageSetting() {
  const { t } = useTranslation();
  const [languagePreference, setLanguagePreference] = useState(
    readStoredLanguagePreference,
  );

  return (
    <SettingsOptionGroup className="mt-8">
      <SettingsOptionRow>
        <div className="min-w-0">
          <label className="text-sm font-medium" htmlFor="app-language">
            {t("settings.language.label")}
          </label>
          <p className="text-sm font-normal text-muted-foreground">
            {t("settings.language.description")}
          </p>
        </div>
        <select
          className="h-8 min-w-36 rounded-full border border-border/50 bg-muted/45 px-3 text-xs font-medium text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="app-language"
          id="app-language"
          onChange={(event) => {
            const nextPreference = event.currentTarget.value;
            if (isAppLanguagePreference(nextPreference)) {
              setLanguagePreference(nextPreference);
              void setAppLanguage(nextPreference);
            }
          }}
          value={languagePreference}
        >
          <option value="system">{t("settings.language.system")}</option>
          <option value="en">{t("settings.language.english")}</option>
          <option value="zh-Hant">
            {t("settings.language.traditionalChinese")}
          </option>
          <option value="zh-Hans">
            {t("settings.language.simplifiedChinese")}
          </option>
        </select>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
