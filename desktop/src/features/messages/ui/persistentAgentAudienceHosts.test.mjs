import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

async function source(relativePath) {
  return readFile(new URL(relativePath, import.meta.url), "utf8");
}

test("only thread conversation hosts opt into persistent audiences", async () => {
  const [channelPane, threadPanel, newMessage, inboxDetail] = await Promise.all(
    [
      source("../../channels/ui/ChannelPane.tsx"),
      source("./MessageThreadPanel.tsx"),
      source("./NewMessageScreen.tsx"),
      source("../../home/ui/InboxDetailPane.tsx"),
    ],
  );

  assert.doesNotMatch(channelPane, /audienceContext=/);
  assert.doesNotMatch(newMessage, /audienceContext=/);
  assert.match(
    threadPanel,
    /audienceContext=\{\{[\s\S]*type: "thread",[\s\S]*rootTags: threadHead\.tags,[\s\S]*\}\}/,
  );
  assert.match(inboxDetail, /type: "thread"/);
  assert.doesNotMatch(threadPanel, /audienceContext=\{[\s\S]*threadRootId/);
  assert.doesNotMatch(inboxDetail, /audienceContext=\{[\s\S]*threadRootId/);
});

test("video review remains explicitly outside persistent audience routing", async () => {
  const videoPlayer = await source("../../../shared/ui/VideoPlayer.tsx");
  const composer = videoPlayer.slice(videoPlayer.indexOf("<MessageComposer"));

  assert.match(composer, /draftKey=/);
  assert.doesNotMatch(
    composer.slice(0, composer.indexOf("/>") + 2),
    /audienceContext=/,
  );
});

test("composer never derives audience context from draft keys", async () => {
  const composer = await source("./MessageComposer.tsx");

  assert.doesNotMatch(composer, /draftKey\?\.startsWith\("thread:"\)/);
  assert.match(composer, /audienceContext && channelId && ownerPubkey/);
});
