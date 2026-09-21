/**
 * Mounted consumer regressions for the join-policy consent block
 * (spec BUZZ-DESKTOP-I18N-ZH-HANS-001, PR 2).
 *
 * The consent sentence is catalogued as fragments so the Terms/Privacy buttons
 * keep their exact DOM. Only a render can prove the fragments reassemble
 * correctly: a stray or missing space changes the accessible label that the
 * invite E2E specs match, and a call site that drifts back to a literal keeps
 * English on screen while every key still resolves.
 */

import assert from "node:assert/strict";
import { after, before, describe, it } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    "buzz-language": "en",
    getItem: (k) => ({ "buzz-language": "en" })[k] ?? null,
    setItem: () => {},
  },
});

Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});

// The consent links open through the OS opener plugin; importing it needs an
// IPC boundary to bind to, and nothing in these tests clicks a link.
globalThis.__TAURI_INTERNALS__ = {
  invoke: () => Promise.reject(new Error("unmocked")),
  transformCallback: () => 1,
};

let React, act, createRoot, JoinPolicyNotice, i18n, language;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  language = await import("@/i18n/language.ts");
  const indexModule = await import("@/i18n/index.ts");
  i18n = indexModule.i18n;
  indexModule.initializeI18n();
  ({ JoinPolicyNotice } = await import("./JoinPolicyNotice.tsx"));
});

after(async () => {
  language.setLanguagePreference("system");
  dom.window.close();
});

const noop = () => {};

function policy({ age = true, terms = true, privacy = true } = {}) {
  return {
    ageAttestationRequired: age,
    termsMarkdown: terms ? "# terms" : null,
    privacyMarkdown: privacy ? "# privacy" : null,
    version: "v1",
  };
}

async function renderNotice(props) {
  const root = createRoot(document.body);
  await act(async () => {
    root.render(
      React.createElement(JoinPolicyNotice, {
        ageConfirmed: false,
        agreementConfirmed: false,
        onAgeConfirmedChange: noop,
        onAgreementConfirmedChange: noop,
        relayWsUrl: "wss://relay.example",
        ...props,
      }),
    );
  });
  return root;
}

function labelTexts() {
  return [...document.querySelectorAll("label")].map(
    (node) => node.textContent ?? "",
  );
}

async function withLanguage(lng, run) {
  await act(async () => {
    language.setLanguagePreference(lng);
  });
  try {
    await run();
  } finally {
    await act(async () => {
      language.setLanguagePreference("en");
    });
  }
}

describe("JoinPolicyNotice consent strings", () => {
  it("reassembles the English consent sentence with single spaces", async () => {
    await withLanguage("en", async () => {
      const root = await renderNotice({ policy: policy() });
      const [ageLabel, consentLabel] = labelTexts();
      assert.equal(ageLabel, "I am 18 years of age or older.");
      assert.equal(
        consentLabel.replace(/\s+/g, " "),
        "I agree to the Buzz Terms of Service and Privacy Policy.",
      );
      assert.match(
        consentLabel,
        /^I agree to the Buzz Terms of Service and Privacy Policy\.$/,
        "fragment joins must not add or drop a space",
      );
      await act(async () => root.unmount());
    });
  });

  it("drops the conjunction when only one policy document exists", async () => {
    await withLanguage("en", async () => {
      const root = await renderNotice({
        policy: policy({ privacy: false }),
      });
      const [, consentLabel] = labelTexts();
      assert.equal(
        consentLabel,
        "I agree to the Buzz Terms of Service.",
        "the conjunction is its own key and must disappear with the second link",
      );
      await act(async () => root.unmount());
    });
  });

  it("renders both consent lines in zh-Hans with no English leftovers", async () => {
    await withLanguage("zh-Hans", async () => {
      const root = await renderNotice({ policy: policy() });
      const [ageLabel, consentLabel] = labelTexts();
      assert.equal(ageLabel, "我已年满 18 周岁。");
      assert.equal(
        consentLabel.replace(/\s+/g, " "),
        "我同意 Buzz服务条款和隐私政策。",
      );
      assert.doesNotMatch(
        consentLabel,
        /Terms of Service|Privacy Policy|Buzz \w+ and/,
      );
      assert.ok(
        i18n.t("onboarding.membership.age-attestation") === ageLabel,
        "the rendered line must come from the active catalog",
      );
      await act(async () => root.unmount());
    });
  });
});
