import { expect, test, type Locator, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

type NostrBindPayload = {
  action: string;
  audience: string;
  challengeId: string;
  expiresAt: string;
  nonce: string;
  origin: string;
  protocol: string;
  returnMode: string;
  verificationCode: string;
  version: string;
};

const VALID_REQUEST: NostrBindPayload = {
  action: "bind_nostr_identity",
  audience: "buzz:nostr-identity",
  challengeId: "550e8400-e29b-41d4-a716-446655440000",
  expiresAt: "2099-01-01T00:00:00Z",
  nonce: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
  origin: "https://admin.example.com",
  protocol: "buzz-nostr-identity",
  returnMode: "clipboard",
  verificationCode: "123456",
  version: "1",
};

async function openNostrBind(
  page: Page,
  payload: NostrBindPayload = VALID_REQUEST,
) {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  await page.evaluate(async (nextPayload) => {
    const internals = (
      window as Window & {
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            args: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    if (!internals?.invoke) {
      throw new Error("Tauri E2E event bridge is unavailable");
    }
    await internals.invoke("plugin:event|emit", {
      event: "deep-link-nostr-bind",
      payload: nextPayload,
    });
  }, payload);

  await expect(page.getByTestId("nostr-bind-page")).toBeVisible();
  await expect(page.getByTestId("nostr-bind-requesting-origin")).toContainText(
    payload.origin,
  );
}

async function pasteCode(input: Locator, code: string) {
  await input.evaluate((element, pastedCode) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text", pastedCode);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      }),
    );
  }, code);
}

async function signCommandPayloads(page: Page): Promise<unknown[]> {
  return page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    )
      .filter(({ command }) => command === "sign_nostr_identity_binding")
      .map(({ payload }) => payload),
  );
}

async function installClipboardStub(page: Page, shouldFail: boolean) {
  await page.addInitScript(
    ({ fail }) => {
      const testWindow = window as Window & {
        __BUZZ_E2E_CLIPBOARD_TEXT__?: string;
      };
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText: async (text: string) => {
            if (fail) {
              throw new Error("clipboard unavailable");
            }
            testWindow.__BUZZ_E2E_CLIPBOARD_TEXT__ = text;
          },
        },
      });
    },
    { fail: shouldFail },
  );
}

async function installShakeCounter(page: Page) {
  await page.addInitScript(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E_CODE_SHAKE_CALLS__?: number;
    };
    testWindow.__BUZZ_E2E_CODE_SHAKE_CALLS__ = 0;
    const originalAnimate = Element.prototype.animate;
    Element.prototype.animate = function animate(keyframes, options) {
      if (
        this instanceof HTMLElement &&
        this.dataset.testid === "nostr-bind-verification-code"
      ) {
        testWindow.__BUZZ_E2E_CODE_SHAKE_CALLS__ =
          (testWindow.__BUZZ_E2E_CODE_SHAKE_CALLS__ ?? 0) + 1;
      }
      return originalAnimate.call(this, keyframes, options);
    };
  });
}

async function shakeCount(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_CODE_SHAKE_CALLS__?: number;
        }
      ).__BUZZ_E2E_CODE_SHAKE_CALLS__ ?? 0,
  );
}

test("shows the exact origin and supports OTP entry, navigation, and paste", async ({
  page,
}) => {
  await openNostrBind(page);

  const first = page.getByTestId("nostr-bind-code-digit-1");
  const second = page.getByTestId("nostr-bind-code-digit-2");
  const third = page.getByTestId("nostr-bind-code-digit-3");
  const continueButton = page.getByTestId("nostr-bind-sign-and-copy");

  await first.click();
  await first.press("1");
  await expect(second).toBeFocused();
  await second.press("2");
  await expect(third).toBeFocused();
  await third.press("ArrowLeft");
  await expect(second).toBeFocused();
  await second.press("Backspace");
  await expect(second).toHaveValue("");
  await expect(continueButton).toBeDisabled();
  await expect.poll(() => signCommandPayloads(page)).toEqual([]);

  await pasteCode(first, VALID_REQUEST.verificationCode);
  for (const [index, digit] of [...VALID_REQUEST.verificationCode].entries()) {
    await expect(
      page.getByTestId(`nostr-bind-code-digit-${index + 1}`),
    ).toHaveValue(digit);
  }
  await expect(page.getByTestId("nostr-bind-code-digit-6")).toBeFocused();
  await expect(continueButton).toBeEnabled();
});

test("locks a filled sixth slot and repeats mismatch feedback without signing", async ({
  page,
}) => {
  await installShakeCounter(page);
  await openNostrBind(page);

  const first = page.getByTestId("nostr-bind-code-digit-1");
  const last = page.getByTestId("nostr-bind-code-digit-6");
  const continueButton = page.getByTestId("nostr-bind-sign-and-copy");

  await pasteCode(first, VALID_REQUEST.verificationCode);
  await last.press("9");
  await expect(last).toHaveValue("6");
  await expect(continueButton).toBeEnabled();
  await expect.poll(() => shakeCount(page)).toBe(1);

  await pasteCode(first, "654321");
  await expect(page.getByRole("alert")).toHaveText(
    "That code doesn't match. Check the code and try again.",
  );
  await expect(continueButton).toBeDisabled();
  await expect.poll(() => shakeCount(page)).toBe(2);

  await pasteCode(first, "654321");
  await expect.poll(() => shakeCount(page)).toBe(3);
  await expect.poll(() => signCommandPayloads(page)).toEqual([]);
});

test("honors reduced motion while rejecting a mismatched code", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installShakeCounter(page);
  await openNostrBind(page);

  await pasteCode(page.getByTestId("nostr-bind-code-digit-1"), "654321");
  await expect(page.getByRole("alert")).toBeVisible();
  await expect.poll(() => shakeCount(page)).toBe(0);
  await expect.poll(() => signCommandPayloads(page)).toEqual([]);
});

test("rejects an expired request without signing", async ({ page }) => {
  await openNostrBind(page, {
    ...VALID_REQUEST,
    expiresAt: "2000-01-01T00:00:00Z",
  });

  await expect(
    page.getByText(
      "This binding link has expired. Request a new one from the requesting app.",
    ),
  ).toBeVisible();
  await pasteCode(
    page.getByTestId("nostr-bind-code-digit-1"),
    VALID_REQUEST.verificationCode,
  );
  await expect(page.getByTestId("nostr-bind-sign-and-copy")).toBeDisabled();
  await expect.poll(() => signCommandPayloads(page)).toEqual([]);
});

test("cancels a request without signing", async ({ page }) => {
  await openNostrBind(page);

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByTestId("nostr-bind-page")).toBeHidden();
  await expect.poll(() => signCommandPayloads(page)).toEqual([]);
});

test("signs a valid request, shows the response, and copies it", async ({
  page,
}) => {
  await installClipboardStub(page, false);
  await openNostrBind(page);
  await pasteCode(
    page.getByTestId("nostr-bind-code-digit-1"),
    VALID_REQUEST.verificationCode,
  );

  await page.getByTestId("nostr-bind-sign-and-copy").click();
  await expect(page.getByTestId("nostr-bind-finish-step")).toBeVisible();
  const response = page.getByTestId("nostr-bind-signed-response");
  await expect(response).toContainText("e2e-signed-nostr-binding");
  await expect
    .poll(() => signCommandPayloads(page))
    .toEqual([
      {
        challengeId: VALID_REQUEST.challengeId,
        expiresAt: VALID_REQUEST.expiresAt,
        nonce: VALID_REQUEST.nonce,
        origin: VALID_REQUEST.origin,
        verificationCode: VALID_REQUEST.verificationCode,
      },
    ]);

  const signedResponse = await response.textContent();
  await page.getByTestId("nostr-bind-copy-response").click();
  await expect(
    page.getByTestId("nostr-bind-copy-response"),
  ).toHaveAccessibleName("Copied");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_CLIPBOARD_TEXT__?: string;
            }
          ).__BUZZ_E2E_CLIPBOARD_TEXT__,
      ),
    )
    .toBe(signedResponse);
});

test("keeps the signed response available when clipboard access fails", async ({
  page,
}) => {
  await installClipboardStub(page, true);
  await openNostrBind(page);
  await pasteCode(
    page.getByTestId("nostr-bind-code-digit-1"),
    VALID_REQUEST.verificationCode,
  );
  await page.getByTestId("nostr-bind-sign-and-copy").click();

  await page.getByTestId("nostr-bind-copy-response").click();
  await expect(
    page.getByText("Buzz couldn't access the clipboard. Try again."),
  ).toBeVisible();
  await expect(page.getByTestId("nostr-bind-signed-response")).toContainText(
    "e2e-signed-nostr-binding",
  );
  await expect(
    page.getByTestId("nostr-bind-copy-response"),
  ).toHaveAccessibleName("Copy response");
});
