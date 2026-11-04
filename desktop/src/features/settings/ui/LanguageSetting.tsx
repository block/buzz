import { useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";

import {
  isAppLanguagePreference,
  getLanguagePreferenceState,
  setAppLanguage,
  subscribeLanguagePreference,
  type AppLanguagePreference,
} from "@/i18n";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function LanguageSetting() {
  const { t } = useTranslation();
  const {
    preference: languagePreference,
    saved,
    nativeSynced,
    changeFailed,
    failedPreference,
  } = useSyncExternalStore(
    subscribeLanguagePreference,
    getLanguagePreferenceState,
    getLanguagePreferenceState,
  );
  const [pending, setPending] = useState(false);
  const changeLanguage = (preference: AppLanguagePreference) => {
    setPending(true);
    void setAppLanguage(preference).then(
      () => setPending(false),
      () => setPending(false),
    );
  };

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
          {!saved ? (
            <p role="status">{t("settings.language.unsaved")}</p>
          ) : null}
          {changeFailed ? (
            <p role="alert">{t("settings.language.error")}</p>
          ) : null}
          {!nativeSynced ? (
            <p role="alert">{t("settings.language.nativeError")}</p>
          ) : null}
          {!saved || changeFailed || !nativeSynced ? (
            <Button
              className="mt-2"
              data-testid="app-language-retry"
              disabled={pending}
              onClick={() =>
                changeLanguage(failedPreference ?? languagePreference)
              }
              size="sm"
              variant="outline"
            >
              {t("settings.language.retry")}
            </Button>
          ) : null}
        </div>
        <select
          className="h-8 min-w-36 rounded-full border border-border/50 bg-muted/45 px-3 text-xs font-medium text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="app-language"
          disabled={pending}
          id="app-language"
          onChange={(event) => {
            const nextPreference = event.currentTarget.value;
            if (isAppLanguagePreference(nextPreference)) {
              changeLanguage(nextPreference);
            }
          }}
          value={languagePreference}
        >
          <option value="system">{t("settings.language.system")}</option>
          <option value="en">{t("settings.language.english")}</option>
          <option value="ko">한국어</option>
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
