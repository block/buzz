import assert from "node:assert/strict";
import test from "node:test";

import { parsePublicWorldViewReference } from "./worldViewDescriptor.ts";

test("parses a public hosted view link without a hand-entered selection", () => {
  const result = parsePublicWorldViewReference(
    "https://manifest.shivai.space/world/exports/public-view-token",
  );

  assert.deepEqual(result, {
    ok: true,
    value: {
      reference: {
        kind: "hosted-world-view-export",
        origin: "https://manifest.shivai.space",
        shareToken: "public-view-token",
      },
      selection: null,
    },
  });
});

test("parses a stable hosted live-view link", () => {
  const result = parsePublicWorldViewReference(
    "https://manifest.shivai.space/world/live/public-live-token",
  );

  assert.deepEqual(result, {
    ok: true,
    value: {
      reference: {
        kind: "hosted-world-live-view-share",
        origin: "https://manifest.shivai.space",
        shareToken: "public-live-token",
      },
      selection: null,
    },
  });
});

test("decodes an encoded token from a loopback development link", () => {
  const result = parsePublicWorldViewReference(
    "http://127.0.0.1:3000/world/exports/public%2Fview%20token",
  );

  assert.deepEqual(result, {
    ok: true,
    value: {
      reference: {
        kind: "hosted-world-view-export",
        origin: "http://127.0.0.1:3000",
        shareToken: "public/view token",
      },
      selection: null,
    },
  });
});

test("parses a copied read-only hosted view reference", () => {
  const result = parsePublicWorldViewReference(`Shivai view reference
Source: hosted view export "public-view-token"
Realm: board::main
View qualified: @main::board`);

  assert.deepEqual(result, {
    ok: true,
    value: {
      reference: {
        kind: "hosted-world-view-export",
        origin: "https://manifest.shivai.space",
        shareToken: "public-view-token",
      },
      selection: {
        realmQualifiedName: "board::main",
        viewQualifiedName: "@main::board",
      },
    },
  });
});

test("rejects local paths without echoing the path", () => {
  const privatePath = "/Users/alice/private/project.world";
  const result = parsePublicWorldViewReference(`Shivai view reference
Source: local world "${privatePath}"
Realm: world::main
View qualified: @main::Board`);

  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.match(result.error, /Local paths cannot be published/);
    assert.doesNotMatch(result.error, /alice|private|project\.world/);
  }
});

test("rejects edit-share capabilities without echoing the token", () => {
  const editToken = "edit-secret-token";
  const result = parsePublicWorldViewReference(`Shivai view reference
Source: hosted edit share "${editToken}"
Realm: world::main
View qualified: @main::Board`);

  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.match(result.error, /Edit-share capabilities cannot be published/);
    assert.doesNotMatch(result.error, /edit-secret-token/);
  }
});
