import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function installAudioCaptureMock(page: Page) {
  await page.addInitScript(() => {
    class FakeAudioTrack {
      stop() {}
    }

    class FakeMediaStream {
      constructor(private readonly tracks = [new FakeAudioTrack()]) {}
      getAudioTracks() {
        return this.tracks;
      }
      getTracks() {
        return this.tracks;
      }
    }

    class FakeAudioNode {
      connect() {
        return this;
      }
      disconnect() {}
    }

    class FakeAudioContext {
      state = "running";
      audioWorklet = { addModule: async () => {} };
      createGain() {
        return Object.assign(new FakeAudioNode(), { gain: { value: 1 } });
      }
      createMediaStreamSource() {
        return new FakeAudioNode();
      }
      async close() {}
      async resume() {}
    }

    class FakeAudioWorkletNode extends FakeAudioNode {
      port = {
        onmessage: null,
        postMessage() {},
      };
    }

    Object.defineProperty(window, "MediaStream", {
      configurable: true,
      value: FakeMediaStream,
    });
    Object.defineProperty(window, "AudioContext", {
      configurable: true,
      value: FakeAudioContext,
    });
    Object.defineProperty(window, "AudioWorkletNode", {
      configurable: true,
      value: FakeAudioWorkletNode,
    });
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: true,
      value: async () => new FakeMediaStream(),
    });
  });
}

async function emitDictationTranscript(page: Page, text: string) {
  await page.evaluate(async (transcript) => {
    const internals = (
      window as Window & {
        __TAURI_INTERNALS__?: {
          invoke: (command: string, payload: unknown) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    if (!internals) throw new Error("mock Tauri internals are unavailable");
    await internals.invoke("plugin:event|emit", {
      event: "dictation-transcript",
      payload: { sessionId: 1, text: transcript },
    });
  }, text);
}

test("dictation inserts finalized speech and waits for Stop before sending", async ({
  page,
}) => {
  await installAudioCaptureMock(page);
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("Existing draft");

  await page.getByRole("button", { name: "Dictate message" }).click();
  await expect(
    page.getByRole("button", { name: "Stop dictation" }),
  ).toBeVisible();
  await expect(page.getByTestId("send-message")).toBeDisabled();

  await emitDictationTranscript(page, "dictated continuation.");
  await expect(input).toContainText("Existing draft dictated continuation.");

  await page.getByRole("button", { name: "Stop dictation" }).click();
  await expect(
    page.getByRole("button", { name: "Finishing dictation" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Dictate message" }),
  ).toBeVisible();
  await expect(page.getByTestId("send-message")).toBeEnabled();
});
