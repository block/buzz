import assert from "node:assert/strict";
import test from "node:test";

const { authoritativeEnterpriseProfile } = await import(
  "./enterpriseProfile.ts"
);

test("authoritativeEnterpriseProfile uses only explicit adapter profile projection", () => {
  assert.deepEqual(
    authoritativeEnterpriseProfile({
      profileProjection: {
        username: " seiler ",
        displayName: " Brad Seiler ",
      },
      expiresAt: "2026-09-18T21:00:00Z",
    }),
    { username: "seiler", displayName: "Brad Seiler" },
  );
  assert.equal(
    authoritativeEnterpriseProfile({
      profileProjection: { username: "seiler", displayName: "   " },
      expiresAt: "2026-09-18T21:00:00Z",
    }),
    null,
  );
  assert.equal(
    authoritativeEnterpriseProfile({
      email: "seiler@example.com",
      expiresAt: "2026-09-18T21:00:00Z",
    }),
    null,
  );
});
