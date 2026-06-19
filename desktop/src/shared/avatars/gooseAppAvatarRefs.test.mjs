import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveImportedPersonaAvatarUrl,
  toGooseAppAvatarRef,
} from "./gooseAppAvatarRefs.ts";

test("toGooseAppAvatarRef canonicalizes app-avatar refs", () => {
  assert.equal(
    toGooseAppAvatarRef("app-avatar:gloopies-19"),
    "app-avatar:gloopies-19",
  );
});

test("toGooseAppAvatarRef detects Goose avatar ids in paths", () => {
  assert.equal(
    toGooseAppAvatarRef("./avatars/pollies_2.png"),
    "app-avatar:pollies-2",
  );
});

test("resolveImportedPersonaAvatarUrl prefers app-avatar refs over data URLs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: "https://example.com/avatar.png",
      avatarRef: "app-avatar:fuzzies-1",
    }),
    "app-avatar:fuzzies-1",
  );
});

test("resolveImportedPersonaAvatarUrl preserves ordinary image URLs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: "https://example.com/avatar.png",
      avatarRef: null,
    }),
    "https://example.com/avatar.png",
  );
});
