import { useTranslation } from "react-i18next";

import { isAppLanguage, resolveAppLanguage, setAppLanguage } from "@/i18n";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function LanguageSetting() {
  const { i18n, t } = useTranslation();
  const language = resolveAppLanguage(i18n.resolvedLanguage);

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
            const nextLanguage = event.currentTarget.value;
            if (isAppLanguage(nextLanguage)) {
              void setAppLanguage(nextLanguage);
            }
          }}
          value={language}
        >
          <option value="en">{t("settings.language.english")}</option>
          <option value="zh-Hant">繁體中文</option>
          <option value="zh-Hans">简体中文</option>
        </select>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
