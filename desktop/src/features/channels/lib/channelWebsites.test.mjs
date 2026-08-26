import assert from "node:assert/strict";
import {
  channelWebsiteFaviconUrl,
  channelWebsiteTabLabel,
  isBlockedEmbedLocation,
  normalizeChannelWebsiteUrl,
  parseChannelWebsitesContent,
  serializeChannelWebsites,
  validateChannelWebsiteDraft,
} from "./channelWebsites.ts";

assert.deepEqual(parseChannelWebsitesContent(""), []);
assert.deepEqual(
  parseChannelWebsitesContent(
    JSON.stringify({
      websites: [
        { id: "a", title: "Docs", url: "https://example.com/docs" },
        { id: "b", title: "", url: "example.org" },
      ],
    }),
  ),
  [
    { id: "a", title: "Docs", url: "https://example.com/docs" },
    { id: "b", title: "", url: "https://example.org" },
  ],
);

assert.equal(
  channelWebsiteTabLabel({ id: "a", title: "Docs", url: "https://x" }),
  "Docs",
);
assert.equal(
  channelWebsiteTabLabel({ id: "a", title: "", url: "https://example.com" }),
  "example.com",
);

assert.equal(normalizeChannelWebsiteUrl("javascript:alert(1)"), null);
assert.equal(
  normalizeChannelWebsiteUrl("https://good.example/path"),
  "https://good.example/path",
);

const draft = validateChannelWebsiteDraft({
  title: "API",
  url: "api.example.com",
});
assert.ok(draft);
assert.equal(draft?.url, "https://api.example.com");

const urlInTitle = validateChannelWebsiteDraft({
  title: "https://www.google.com",
  url: "",
});
assert.ok(urlInTitle);
assert.equal(urlInTitle?.url, "https://www.google.com");
assert.equal(urlInTitle?.title, "");

assert.equal(validateChannelWebsiteDraft({ title: "", url: "" }), null);

assert.equal(isBlockedEmbedLocation("about:blank"), true);
assert.equal(isBlockedEmbedLocation("chrome-error://chromewebdata/"), true);
assert.equal(isBlockedEmbedLocation("https://example.com"), false);

assert.equal(
  channelWebsiteFaviconUrl("https://example.com/docs"),
  "https://www.google.com/s2/favicons?domain=example.com&sz=32",
);
assert.equal(channelWebsiteFaviconUrl("not a url"), null);

const roundtrip = serializeChannelWebsites([
  { id: "1", title: "One", url: "https://one.example" },
]);
assert.equal(parseChannelWebsitesContent(roundtrip).length, 1);

console.log("channelWebsites.test.mjs: ok");
