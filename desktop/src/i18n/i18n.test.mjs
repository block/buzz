/**
 * i18n singleton tests (spec T-003 / T-004 / T-005): runtime switching
 * keeps rendered text and <html lang> in sync without a reload, missing or
 * empty zh values fall back to English, and the two catalogs stay in parity.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

function setNavigatorLanguages(languages) {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages, userAgent: "buzz-unit-test" },
  });
}

function installLocalStorage(entries = {}) {
  const store = new Map(Object.entries(entries));
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  return store;
}

let React;
let rtl;
let language;
let i18n;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
  installLocalStorage();
  setNavigatorLanguages(["zh-CN"]);

  React = await import("react");
  rtl = await import("@testing-library/react");
  language = await import("./language.ts");
  const indexModule = await import("./index.ts");
  i18n = indexModule.i18n;
  indexModule.initializeI18n();
});

afterEach(async () => {
  rtl.cleanup();
});

after(() => dom.window.close());

// T-003 / FR-001: a zh system with no stored preference boots into
// zh-Hans; an explicit switch updates rendered text and <html lang>
// instantly, and persists for the next boot.
test("zh system boots into zh-Hans; live switch updates text and html lang", async () => {
  const { useTranslation } = await import("react-i18next");
  function Probe() {
    const { t } = useTranslation();
    return React.createElement(
      "span",
      { "data-testid": "probe" },
      t("settings.appearance.language.label"),
    );
  }

  const view = rtl.render(React.createElement(Probe));
  void view;

  assert.equal(
    rtl.screen.getByTestId("probe").textContent,
    "语言",
    "first paint must already be in the resolved language",
  );
  assert.equal(document.documentElement.lang, "zh-Hans");

  await rtl.act(async () => {
    language.setLanguagePreference("en");
  });
  assert.equal(rtl.screen.getByTestId("probe").textContent, "Language");
  assert.equal(document.documentElement.lang, "en");
  assert.equal(globalThis.localStorage.getItem("buzz-language"), "en");
});

test("an explicit preference survives a storage re-read", () => {
  installLocalStorage({ "buzz-language": "zh-Hans" });
  language.initializeLanguagePreference();
  assert.equal(language.getLanguagePreference(), "zh-Hans");
  assert.equal(language.getEffectiveLanguage(), "zh-Hans");
  assert.equal(document.documentElement.lang, "zh-Hans");
});

test("switching back to system follows the system locale again", async () => {
  installLocalStorage({ "buzz-language": "system" });
  setNavigatorLanguages(["fr", "zh-SG"]);
  language.initializeLanguagePreference();
  assert.equal(language.getEffectiveLanguage(), "zh-Hans");

  await rtl.act(async () => {
    language.setLanguagePreference("system");
  });
  assert.equal(i18n.language, "zh-Hans");
});

// T-004 / FR-003: missing or empty zh-Hans values surface the English
// translation — never the key, never a blank.
test("missing or empty zh-Hans values fall back to English", async () => {
  const i18next = (await import("i18next")).default;
  const en = (await import("../locales/en.json")).default;
  const instance = i18next.createInstance();
  const partialZh = structuredClone(en);
  delete partialZh.settings.appearance.language.option.en; // missing leaf
  partialZh.settings.appearance.font.size.label = ""; // empty value

  await instance.init({
    fallbackLng: "en",
    supportedLngs: ["en", "zh-Hans"],
    load: "currentOnly",
    lng: "zh-Hans",
    resources: {
      en: { translation: en },
      "zh-Hans": { translation: partialZh },
    },
    returnEmptyString: false,
    interpolation: { escapeValue: false },
    initAsync: false,
  });

  assert.equal(
    instance.t("settings.appearance.language.option.en", { lng: "zh-Hans" }),
    "English",
  );
  assert.equal(
    instance.t("settings.appearance.font.size.label", { lng: "zh-Hans" }),
    "Font size",
  );
});

// T-005 / NFR-002: the shipped catalogs are in key and variable parity.
test("shipped en and zh-Hans catalogs are in parity", async () => {
  const { loadCatalogs, validateCatalogParity } = await import(
    "./catalogValidation.mjs"
  );
  const { en, zhHans } = loadCatalogs();
  assert.deepEqual(validateCatalogParity(en, zhHans), []);
});

test("catalog validation catches key, empty-value, and extra-key drift", async () => {
  const { loadCatalogs, validateCatalogParity } = await import(
    "./catalogValidation.mjs"
  );
  const { en, zhHans } = loadCatalogs();

  const mutated = structuredClone(zhHans);
  delete mutated.settings.appearance.language.option.en;
  mutated.settings.appearance.language.description = "";
  mutated.settings.appearance.language.extraKey = "surprise";

  const problems = validateCatalogParity(en, mutated);
  assert(
    problems.some((p) =>
      p.includes("missing zh-Hans key: settings.appearance.language.option.en"),
    ),
    problems.join("\n"),
  );
  assert(
    problems.some((p) =>
      p.includes(
        "empty or non-string zh-Hans value: settings.appearance.language.description",
      ),
    ),
    problems.join("\n"),
  );
  assert(
    problems.some((p) =>
      p.includes(
        "unexpected zh-Hans key (not in en source): settings.appearance.language.extraKey",
      ),
    ),
    problems.join("\n"),
  );
});

test("interpolation variable drift is reported", async () => {
  const { validateCatalogParity } = await import("./catalogValidation.mjs");
  const en = {
    greeting: { name: "Hello {{name}}, {{count}} items" },
  };
  const zhHans = {
    greeting: { name: "你好 {{name}}，共 {{count}} 条" },
  };
  assert.deepEqual(validateCatalogParity(en, zhHans), []);

  const drifted = { greeting: { name: "你好 {{name}}" } };
  assert(
    validateCatalogParity(en, drifted).some((p) =>
      p.includes("interpolation variable drift"),
    ),
  );
});

// A key renamed on the component side only renders the raw dotted key to the
// user, and no catalog-to-catalog comparison can see it.
test("every literal t() call site resolves in both catalogs", async () => {
  const {
    collectTranslationCallSites,
    loadCatalogs,
    validateTranslationCallSites,
  } = await import("./catalogValidation.mjs");
  const { en, zhHans } = loadCatalogs();
  const sites = collectTranslationCallSites();

  // Without this, a source walk that silently finds nothing would pass.
  assert(
    sites.has("onboarding.setup.title-harness"),
    `source walk found ${sites.size} keys; expected the shipped onboarding keys`,
  );
  assert.deepEqual(validateTranslationCallSites(en, zhHans), []);
});

test("the call-site guard catches a key renamed on the component side only", async () => {
  const { loadCatalogs, validateTranslationCallSites } = await import(
    "./catalogValidation.mjs"
  );
  const { en, zhHans } = loadCatalogs();
  const usedKey = "onboarding.setup.title-harness";

  const mutated = structuredClone(en);
  delete mutated.onboarding.setup[usedKey.split(".").pop()];

  const problems = validateTranslationCallSites(mutated, zhHans);
  assert.equal(problems.length, 1, problems.join("\n"));
  assert(problems[0].includes(`unresolved translation key: ${usedKey}`));
  assert(problems[0].includes("features/onboarding/ui/SetupStep.tsx"));
});

test("a plural call site needs both forms and renders for either count", async () => {
  const { loadCatalogs, validateTranslationCallSites } = await import(
    "./catalogValidation.mjs"
  );
  const { en, zhHans } = loadCatalogs();
  const base = "sidebar.rail.mention-count";

  assert.deepEqual(validateTranslationCallSites(en, zhHans), []);

  const halfDeclared = structuredClone(en);
  delete halfDeclared.sidebar.rail["mention-count_other"];
  const problems = validateTranslationCallSites(halfDeclared, zhHans);
  assert.equal(problems.length, 1, problems.join("\n"));
  assert(problems[0].includes(`unresolved translation key: ${base}`));
  assert(problems[0].includes("missing in en"));

  await i18n.changeLanguage("en");
  assert.equal(i18n.t(base, { community: "Lab", count: 1 }), "Lab — 1 mention");
  assert.equal(
    i18n.t(base, { community: "Lab", count: 3 }),
    "Lab — 3 mentions",
  );
  await i18n.changeLanguage("zh-Hans");
  assert.equal(i18n.t(base, { community: "Lab", count: 1 }), "Lab — 1 条提及");
  assert.equal(i18n.t(base, { community: "Lab", count: 3 }), "Lab — 3 条提及");
  await i18n.changeLanguage("en");
});

test("dynamic translation keys need a declared exception with every form", async () => {
  const {
    DYNAMIC_KEY_EXCEPTIONS,
    collectDynamicKeySites,
    loadCatalogs,
    validateDynamicKeys,
  } = await import("./catalogValidation.mjs");
  const { en, zhHans } = loadCatalogs();
  const sites = collectDynamicKeySites();
  const prefix = "onboarding.backup.separator-";

  // The shipped exception is the only dynamic call site in the source tree.
  assert.deepEqual(
    validateDynamicKeys(en, zhHans, DYNAMIC_KEY_EXCEPTIONS, sites),
    [],
  );
  assert.deepEqual(
    [...sites.keys()],
    [prefix],
    "an undeclared dynamic t() call site must fail the gate",
  );

  const missingForm = structuredClone(en);
  delete missingForm.onboarding.backup["separator-commas"];
  const missing = validateDynamicKeys(
    missingForm,
    zhHans,
    DYNAMIC_KEY_EXCEPTIONS,
    sites,
  );
  assert.equal(missing.length, 1, missing.join("\n"));
  assert(
    missing[0].startsWith(
      `unresolved dynamic translation key: ${prefix}commas`,
    ),
  );
  assert(missing[0].includes("missing in en"));

  const undeclaredSite = new Map([["settings.something-", ["x.test.tsx:1"]]]);
  const undeclared = validateDynamicKeys(
    en,
    zhHans,
    DYNAMIC_KEY_EXCEPTIONS,
    undeclaredSite,
  );
  assert(
    undeclared.some((problem) =>
      problem.startsWith(
        "undeclared dynamic translation key: t(`settings.something-...`)",
      ),
    ),
    undeclared.join("\n"),
  );
  assert(
    undeclared.some((problem) =>
      problem.startsWith("stale dynamic translation exception"),
    ),
    undeclared.join("\n"),
  );

  const extraForm = structuredClone(en);
  extraForm.onboarding.backup["separator-underscores"] = "_";
  const extraZh = structuredClone(zhHans);
  extraZh.onboarding.backup["separator-underscores"] = "_";
  const widened = validateDynamicKeys(
    extraForm,
    extraZh,
    DYNAMIC_KEY_EXCEPTIONS,
    sites,
  );
  assert.equal(widened.length, 1, widened.join("\n"));
  assert(
    widened[0].startsWith(
      `undeclared dynamic translation form: ${prefix}underscores`,
    ),
  );
});

test("a key named in a prose comment is not counted as a call site", async () => {
  const { TRANSLATION_CALL_RE, withoutWholeLineComments } = await import(
    "./catalogValidation.mjs"
  );
  const fixture = [
    '  // grouped above the t("onboarding.setup.this-key-does-not-exist")',
    "  const heading = t(",
    '    "onboarding.setup.title-harness",',
    "  );",
  ].join("\n");

  const stripped = withoutWholeLineComments(fixture);
  assert.equal(
    stripped.split("\n").length,
    fixture.split("\n").length,
    "stripping must keep one line per input line so reported line numbers hold",
  );

  const found = [...stripped.matchAll(TRANSLATION_CALL_RE)].map((m) => m[1]);
  assert.deepEqual(found, ["onboarding.setup.title-harness"]);
});
