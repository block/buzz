import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { takeQueuedAttachmentsForDraft } from "@/features/messages/lib/backgroundMediaUploadStore";
import { initDraftStore } from "@/features/messages/lib/useDrafts";

import { installLocalStorage } from "./testStorage.mjs";
import {
  dismissAndStagePlaygroundScreenshot,
  playgroundScreenshotFile,
  stagePlaygroundScreenshotDraft,
} from "./screenshot.ts";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "1234",
  sid: "demo-1",
};

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetPlaygroundState } = await import("./sessions.ts");
  resetPlaygroundState();
  globalThis.localStorage?.clear();
});

test("screenshot dismisses and stages a draft attachment", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
  } = await import("./sessions.ts");
  configurePlaygroundScope("pub", "wss://relay.example.com");
  initDraftStore("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  assert.equal(getActivePlaygroundSid(), "demo-1");

  const file = playgroundScreenshotFile({
    bytes: [137, 80, 78, 71],
    mime: "image/png",
    filename: "playground-demo-1.png",
  });
  dismissAndStagePlaygroundScreenshot({
    conversation: { channelId: "hula-id", draftKey: "hula-id" },
    file,
  });
  assert.equal(getActivePlaygroundSid(), null);
  const queued = takeQueuedAttachmentsForDraft("hula-id");
  assert.equal(queued.length, 1);
  assert.equal(queued[0]?.file.name, "playground-demo-1.png");
});

test("chrome screenshot stages draft without dismissing the webview host", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
  } = await import("./sessions.ts");
  configurePlaygroundScope("pub", "wss://relay.example.com");
  initDraftStore("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  assert.equal(getActivePlaygroundSid(), "demo-1");

  const file = playgroundScreenshotFile({
    bytes: [137, 80, 78, 71],
    mime: "image/png",
    filename: "playground-demo-1.png",
  });
  stagePlaygroundScreenshotDraft({
    conversation: { channelId: "hula-id", draftKey: "hula-id" },
    file,
  });
  // Split/dock must keep rendering — staging alone must not park the host.
  assert.equal(getActivePlaygroundSid(), "demo-1");
  const queued = takeQueuedAttachmentsForDraft("hula-id");
  assert.equal(queued.length, 1);
  assert.equal(queued[0]?.file.name, "playground-demo-1.png");
});
