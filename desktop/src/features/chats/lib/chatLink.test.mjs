import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChatLink,
  isChatLink,
  parseChatLink,
  parseChatRouteLink,
  resolveChatLinkRenderTarget,
} from "./chatLink.ts";

const CHAT = "f570339f-8f8a-4e08-a779-8d954aa44109";

test("buildChatLink -> parseChatLink round-trips with title", () => {
  const url = buildChatLink({ chatId: CHAT, title: "Corner radius notes" });
  assert.equal(url, `buzz://chat?channel=${CHAT}&title=Corner+radius+notes`);

  const parsed = parseChatLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    chatId: CHAT,
    title: "Corner radius notes",
  });
});

test("buildChatLink omits blank titles", () => {
  assert.equal(
    buildChatLink({ chatId: CHAT, title: " " }),
    `buzz://chat?channel=${CHAT}`,
  );
});

test("buildChatLink rejects missing chat id", () => {
  assert.throws(() => buildChatLink({ chatId: "" }));
});

test("parseChatLink rejects unsupported links", () => {
  assert.deepEqual(parseChatLink(`https://example.com/?channel=${CHAT}`), {
    ok: false,
    reason: "wrong-scheme",
  });
  assert.deepEqual(parseChatLink("buzz://message?channel=c&id=m"), {
    ok: false,
    reason: "wrong-host",
  });
  assert.deepEqual(parseChatLink("buzz://chat"), {
    ok: false,
    reason: "missing-channel",
  });
});

test("parseChatRouteLink accepts chat routes", () => {
  assert.deepEqual(parseChatRouteLink(`/chats/${CHAT}`), {
    ok: true,
    value: {
      chatId: CHAT,
      title: null,
    },
  });
  assert.deepEqual(
    parseChatRouteLink(
      `https://buzz.test/chats/${CHAT}?title=Corner+radius+notes`,
      {
        currentOrigin: "https://buzz.test",
      },
    ),
    {
      ok: true,
      value: {
        chatId: CHAT,
        title: "Corner radius notes",
      },
    },
  );
});

test("parseChatRouteLink rejects non-app absolute links", () => {
  assert.deepEqual(parseChatRouteLink(`https://buzz.test/chats/${CHAT}`), {
    ok: false,
    reason: "wrong-origin",
  });
  assert.deepEqual(
    parseChatRouteLink(`https://example.com/chats/${CHAT}`, {
      currentOrigin: "https://buzz.test",
    }),
    {
      ok: false,
      reason: "wrong-origin",
    },
  );
  assert.deepEqual(parseChatRouteLink("/channels/general"), {
    ok: false,
    reason: "wrong-path",
  });
});

test("isChatLink matches chat links only", () => {
  assert.equal(isChatLink(`buzz://chat?channel=${CHAT}`), true);
  assert.equal(isChatLink("buzz://message?channel=c&id=m"), false);
  assert.equal(isChatLink("https://example.com"), false);
  assert.equal(isChatLink(undefined), false);
  assert.equal(isChatLink(""), false);
});

test("resolveChatLinkRenderTarget distinguishes cards from labels", () => {
  const href = buildChatLink({ chatId: CHAT, title: "Build notes" });

  assert.deepEqual(resolveChatLinkRenderTarget({ href, label: href }), {
    kind: "card",
    link: {
      chatId: CHAT,
      title: "Build notes",
    },
  });
  assert.deepEqual(resolveChatLinkRenderTarget({ href, label: "Open chat" }), {
    kind: "label",
    link: {
      chatId: CHAT,
      title: "Build notes",
    },
  });
  assert.deepEqual(
    resolveChatLinkRenderTarget({
      href: "https://example.com",
      label: href,
    }),
    { kind: "none" },
  );
});

test("resolveChatLinkRenderTarget routes app chat links", () => {
  assert.deepEqual(
    resolveChatLinkRenderTarget({
      href: `/chats/${CHAT}`,
      label: `/chats/${CHAT}`,
    }),
    {
      kind: "card",
      link: {
        chatId: CHAT,
        title: null,
      },
    },
  );
  assert.deepEqual(
    resolveChatLinkRenderTarget({
      href: `/chats/${CHAT}?title=Build+notes`,
      label: "Open chat",
    }),
    {
      kind: "label",
      link: {
        chatId: CHAT,
        title: "Build notes",
      },
    },
  );
});
