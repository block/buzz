import { expect, test } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

declare global {
  interface Window {
    __AUTOFOCUS_ORDER__: {
      held: number;
      delivered: number;
      release: () => void;
    };
  }
}

for (const claimed of [true, false]) {
  test(`late automatic focus ${claimed ? "respects an open native menu" : "focuses an unclaimed composer"}`, async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const nativeRAF = window.requestAnimationFrame.bind(window);
      const held: FrameRequestCallback[] = [];
      const state = {
        held: 0,
        delivered: 0,
        release: () => {
          for (const callback of held.splice(0))
            nativeRAF((time) => {
              callback(time);
              state.delivered++;
            });
        },
      };
      window.__AUTOFOCUS_ORDER__ = state;
      window.requestAnimationFrame = (callback) => {
        // Source-bound to scheduleComposerAutofocus's commit (selection +
        // scroll). Hold the genuine callback only when the browser delivers it;
        // other frames, including Tiptap explicit focus, are forwarded unchanged.
        const source = callback.toString();
        const automatic =
          source.includes(".state.tr.setSelection(") &&
          source.includes(".commands.scrollIntoView()");
        return nativeRAF((time) => {
          if (automatic) {
            held.push(callback);
            state.held++;
          } else callback(time);
        });
      };
    });
    await installMockBridge(page, {
      windowLabel: "huddle-11111111-1111-4111-8111-111111111111",
      ttsSettings: {
        version: 1,
        agentTextToSpeech: true,
        voicePreferences: ["pocket:vera"],
      },
      huddle: {
        parentChannelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        ephemeralChannelId: "11111111-1111-4111-8111-111111111111",
        members: [
          { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
          { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
        ],
        ttsEnabled: true,
      },
    });
    await page.goto("/");
    await expect
      .poll(() => page.evaluate(() => window.__AUTOFOCUS_ORDER__.held))
      .toBeGreaterThan(0);
    const editor = page.getByTestId("message-input");
    await expect(editor).not.toBeFocused();
    const trigger = page.getByRole("button", {
      name: "Voice settings for alice",
    });
    const menu = page.locator(
      '[data-testid="huddle-agent-voice-menu-content"][data-state="open"]',
    );
    if (claimed) {
      await trigger.click();
      await expect(menu).toBeVisible();
      await expect(menu.getByTestId("huddle-agent-tts-toggle")).toBeFocused();
    }
    await page.evaluate(() => window.__AUTOFOCUS_ORDER__.release());
    await expect
      .poll(() => page.evaluate(() => window.__AUTOFOCUS_ORDER__.delivered))
      .toBeGreaterThan(0);
    if (claimed) {
      await expect(trigger).toHaveAttribute("aria-expanded", "true");
      await expect(menu).toBeVisible();
      await expect(editor).not.toBeFocused();
      await menu.getByTestId("huddle-agent-tts-toggle").click();
      await expect(
        menu.getByTestId("huddle-agent-tts-toggle"),
      ).not.toBeChecked();
      await page.keyboard.press("Escape");
      await editor.click();
      await expect(editor).toBeFocused();
      await editor.fill("explicit typing still works");
      await expect(editor).toHaveText("explicit typing still works");
    } else {
      await expect(editor).toBeFocused();
    }
  });
}
