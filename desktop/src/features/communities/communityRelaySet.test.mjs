import assert from "node:assert/strict";
import test from "node:test";
import { communityRelaySetKey } from "./communityRelaySet.ts";

const communities = (...urls) => urls.map((relayUrl) => ({ relayUrl }));

test("inactive community add/edit/removal changes trust while reorder and names do not", () => {
  const a = "wss://a.example";
  const b = "wss://b.example";
  const key = communityRelaySetKey(communities(a, b));
  assert.equal(
    key,
    communityRelaySetKey(communities(b, a, "https://a.example/")),
  );
  assert.notEqual(key, communityRelaySetKey(communities(b)));
  assert.notEqual(
    key,
    communityRelaySetKey(communities("wss://new-a.example", b)),
  );
  assert.notEqual(
    key,
    communityRelaySetKey(communities(a, b, "wss://c.example")),
  );
  assert.equal(
    key,
    communityRelaySetKey(
      communities(a, b).map((c) => ({ ...c, name: "new label" })),
    ),
  );
});

test("invalid URLs cannot become trusted origins by normalization", () => {
  assert.equal(
    communityRelaySetKey(
      communities(
        "bad",
        "file:///etc",
        "https://user@a.example",
        "wss://a.example/path",
        "https://a.example?q=1",
      ),
    ),
    "[]",
  );
});
