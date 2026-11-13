import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

const utilitiesCss = read("../styles/globals/utilities.css");

// The class substrings the solid fallback rule matches on.
const fallbackSubstrings = [
  ...utilitiesCss.matchAll(
    /:root\[data-backdrop-filter-unpainted\]\s+\[class\*="([^"]+)"\]/g,
  ),
].map((match) => match[1]);

test("the solid fallback is keyed on the root attribute", () => {
  assert.deepEqual(fallbackSubstrings, [
    "backdrop-filter]:bg-background/",
    "backdrop-filter:bg-background/",
  ]);
});

// The surfaces whose see-through tint overlapped text when WebKitGTK ran
// without compositing: the composer, the channel/thread headers, the top chrome.
for (const path of [
  "../../features/messages/ui/MessageComposer.tsx",
  "../../features/channels/ui/ChannelPane.tsx",
  "../../features/chat/ui/ChatHeader.tsx",
  "../layout/AuxiliaryPanelHeader.tsx",
  "../layout/chromeLayout.ts",
  "../ui/TopChromeBackdrop.tsx",
]) {
  test(`solid fallback covers ${path.split("/").at(-1)}`, () => {
    const source = read(path);
    assert.ok(
      fallbackSubstrings.some((substring) => source.includes(substring)),
      "the glass tint no longer matches the fallback rule in utilities.css",
    );
  });
}
