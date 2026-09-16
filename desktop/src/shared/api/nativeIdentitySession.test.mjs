import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bindNativeIdentity,
  nativeGeneration,
  nativeIdentity,
  revokeNativeIdentity,
} from "./nativeIdentitySession.ts";

test("renderer realm cannot adopt a replacement generation or revive revoked authority", () => {
  bindNativeIdentity({
    mode: "local",
    authState: "local",
    publicIdentity: "local",
    generation: 0,
    workspaceActive: true,
  });
  assert.equal(nativeGeneration(), undefined);
  bindNativeIdentity({
    mode: "remote",
    authState: "authenticated",
    publicIdentity: "owner",
    generation: 4,
    workspaceActive: true,
  });
  assert.equal(nativeGeneration(), 4);
  assert.throws(
    () => bindNativeIdentity({ ...nativeIdentity(), generation: 5 }),
    /fresh renderer/,
  );
  revokeNativeIdentity();
  assert.throws(nativeGeneration, /Sign in/);
  bindNativeIdentity({ ...nativeIdentity() });
  assert.throws(nativeGeneration, /Sign in/);
});
