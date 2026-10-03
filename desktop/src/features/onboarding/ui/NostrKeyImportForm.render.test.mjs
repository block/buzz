/**
 * Behavioral regression tests for the file-import raw-nsec Continue button.
 *
 * See PR #5308 and the review comments from @ravarora2 and @tellaho: in
 * mode === "backup", a valid raw nsec sets isValid to true but neither
 * mode === "key" nor isPasswordStage is true, so the old gate
 * `mode === "key" || isPasswordStage` rendered the CTA as null — a dead
 * end. Adding `|| isValid` unblocks it.
 *
 * These tests use @testing-library/react with jsdom to exercise the
 * behavioral regression: simulate selecting a file containing a valid raw
 * nsec, verify the identity preview and submit CTA appear, submit it, and
 * verify the onImport callback fires.
 */
import assert from "node:assert/strict";
import { after, before, beforeEach, afterEach, test } from "node:test";

import { JSDOM } from "jsdom";

let act;
let cleanup;
let createElement;
let fireEvent;
let render;
let waitFor;
let NostrKeyImportForm;

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    FileList: dom.window.FileList,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });

  // Do NOT override Node's native File — jsdom's File lacks `.text()`,
  // which the component's handleFiles calls. Node's File has it.

  ({
    act,
    cleanup,
    fireEvent,
    render,
    waitFor,
  } = await import("@testing-library/react"));
  ({ createElement } = await import("react"));
  ({ NostrKeyImportForm } = await import("./NostrKeyImportForm.tsx"));
});

after(() => dom.window.close());
afterEach(async () => {
  cleanup();
});

/**
 * Generate a valid nsec1 key for testing. Uses nostr-tools the same way
 * the existing keyImportInput.test.mjs does.
 */
async function makeValidNsec() {
  const { nsecEncode } = await import("nostr-tools/nip19");
  const { generateSecretKey } = await import("nostr-tools/pure");
  return nsecEncode(generateSecretKey());
}

/**
 * Create a FileList-like object containing a single text file with the
 * given content. jsdom does not implement DataTransfer and its File
 * lacks `.text()`, so we use Node's native File (which has `.text()`)
 * and build a FileList shape directly.
 */
function makeFileList(content, name = "identity.key") {
  const file = new File([content], name, { type: "text/plain" });
  const list = Object.create(FileList.prototype);
  Object.defineProperties(list, {
    0: { value: file, enumerable: true },
    length: { value: 1, enumerable: true },
    item: { value: (index) => (index === 0 ? file : null), enumerable: true },
  });
  return list;
}

test("backup mode with no input does not render the submit button", () => {
  const { container } = render(
    createElement(NostrKeyImportForm, {
      mode: "backup",
      onBack: () => {},
      onImport: async () => {},
      showBack: false,
    }),
  );
  assert.ok(
    !container.querySelector('[data-testid="nostr-import-submit"]'),
    "submit button must not render in backup mode with no valid key",
  );
});

test("key mode renders the submit button (mode === key short-circuits the guard)", () => {
  const { container } = render(
    createElement(NostrKeyImportForm, {
      mode: "key",
      onBack: () => {},
      onImport: async () => {},
      showBack: false,
    }),
  );
  assert.ok(
    container.querySelector('[data-testid="nostr-import-submit"]'),
    "submit button must render in key mode even without input",
  );
});

test("selecting a file with a valid raw nsec in backup mode shows the identity preview and submit button (regression for #5308)", async () => {
  const validNsec = await makeValidNsec();

  const { container } = render(
    createElement(NostrKeyImportForm, {
      mode: "backup",
      onBack: () => {},
      onImport: async () => {},
      showBack: false,
    }),
  );

  // In backup mode with no input, the submit button must not be present.
  assert.ok(
    !container.querySelector('[data-testid="nostr-import-submit"]'),
    "submit button must not render before a valid key is loaded",
  );

  // Simulate selecting a file containing a valid raw nsec.
  const fileInput = container.querySelector(
    '[data-testid="nostr-import-file-input"]',
  );
  assert.ok(fileInput, "file input must be present in backup mode");

  await act(async () => {
    Object.defineProperty(fileInput, "files", {
      value: makeFileList(validNsec),
      writable: false,
      configurable: true,
    });
    fileInput.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    // handleFiles is async — it calls file.text() which returns a Promise.
    // Flush the microtask queue so the state update is committed before act ends.
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  // After loading a valid nsec, the identity preview must appear.
  await waitFor(() => {
    assert.ok(
      container.querySelector('[data-testid="nostr-import-npub-preview"]'),
      "identity preview must appear after loading a valid raw nsec",
    );
  });

  // The submit button must now be visible and enabled.
  const submitButton = container.querySelector(
    '[data-testid="nostr-import-submit"]',
  );
  assert.ok(submitButton, "submit button must render after loading a valid raw nsec");
  assert.ok(
    !submitButton.disabled,
    "submit button must be enabled when a valid raw nsec is loaded",
  );
});

test("submitting a valid raw nsec in backup mode fires onImport with the key (regression for #5308)", async () => {
  const validNsec = await makeValidNsec();
  let importedNsec = null;

  const { container } = render(
    createElement(NostrKeyImportForm, {
      mode: "backup",
      onBack: () => {},
      onImport: async (nsec) => {
        importedNsec = nsec;
      },
      showBack: false,
    }),
  );

  // Load a file with a valid raw nsec.
  const fileInput = container.querySelector(
    '[data-testid="nostr-import-file-input"]',
  );
  await act(async () => {
    Object.defineProperty(fileInput, "files", {
      value: makeFileList(validNsec),
      writable: false,
      configurable: true,
    });
    fileInput.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  // Wait for the submit button to appear, then click it.
  await waitFor(() => {
    assert.ok(
      container.querySelector('[data-testid="nostr-import-submit"]'),
      "submit button must render after loading a valid raw nsec",
    );
  });

  const submitButton = container.querySelector(
    '[data-testid="nostr-import-submit"]',
  );
  await act(async () => {
    submitButton.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  });

  // Verify the onImport callback received the raw nsec.
  assert.equal(
    importedNsec,
    validNsec,
    "onImport must be called with the raw nsec from the selected file",
  );
});
