import assert from "node:assert/strict";
import test from "node:test";

import { resolveAgentCardAvatarUrl } from "./resolveAgentCardAvatarUrl.ts";

test("stopped legacy agents use the catalog-projected runtime avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl([null, null, "app-avatar://devin"]),
    "app-avatar://devin",
  );
});

test("stored and custom profile avatars outrank the runtime fallback", () => {
  assert.equal(
    resolveAgentCardAvatarUrl([
      " https://relay.example/custom.png ",
      "https://stored.example/custom.png",
      "app-avatar://devin",
    ]),
    "https://relay.example/custom.png",
  );
  assert.equal(
    resolveAgentCardAvatarUrl([
      " ",
      "https://stored.example/custom.png",
      "app-avatar://devin",
    ]),
    "https://stored.example/custom.png",
  );
});

test("superseded relay defaults fall through to the current runtime avatar", () => {
  const legacy = "https://runtime.example/old-default.svg";
  assert.equal(
    resolveAgentCardAvatarUrl(
      [null, legacy, "data:image/svg+xml,current"],
      [legacy],
    ),
    "data:image/svg+xml,current",
  );
});

test("persona callers preserve custom instance avatars before the runtime fallback", () => {
  assert.equal(
    resolveAgentCardAvatarUrl([
      null,
      "https://stored.example/custom.png",
      "/runtime-icons/current.svg",
    ]),
    "https://stored.example/custom.png",
  );
});

test("persona-only cards use the catalog runtime icon before an instance exists", () => {
  assert.equal(
    resolveAgentCardAvatarUrl([
      null,
      "/runtime-icons/devin.svg",
      "data:image/svg+xml,runtime-avatar",
    ]),
    "/runtime-icons/devin.svg",
  );
});

test("missing custom and runtime avatars preserve the initials fallback", () => {
  assert.equal(resolveAgentCardAvatarUrl([undefined, "", null]), null);
});
