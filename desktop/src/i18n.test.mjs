import assert from "node:assert/strict";
import test from "node:test";

import { createAppI18n } from "./i18n.ts";

test("changing language updates translated text without a restart", async () => {
  const instance = createAppI18n(
    {
      en: {
        translation: {
          sample: "English sample",
        },
      },
      "zh-Hant": {
        translation: {
          sample: "Localized sample",
        },
      },
      "zh-Hans": {
        translation: {
          sample: "Another localized sample",
        },
      },
    },
    "en",
  );

  assert.equal(instance.t("sample"), "English sample");

  await instance.changeLanguage("zh-Hant");

  assert.equal(instance.t("sample"), "Localized sample");
});

test("empty Chinese values fall back to English", async () => {
  const instance = createAppI18n(
    {
      en: {
        translation: {
          sample: "English fallback",
        },
      },
      "zh-Hant": {
        translation: {
          sample: "",
        },
      },
      "zh-Hans": {
        translation: {
          sample: "",
        },
      },
    },
    "zh-Hant",
  );

  assert.equal(instance.t("sample"), "English fallback");
});
