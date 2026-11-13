import assert from "node:assert/strict";
import test from "node:test";

import {
  connectInvitedCommunity,
  createCommunity,
  isValidEmail,
  listInvitedCommunities,
  logIn,
  MIN_PASSWORD_LENGTH,
  OnboardingAccountError,
  sendCommunityInvites,
  signUp,
} from "./onboardingAccountStubs.ts";

test("isValidEmail accepts plausible addresses and rejects junk", () => {
  assert.equal(isValidEmail("cynthia@example.com"), true);
  assert.equal(isValidEmail("  padded@example.com  "), true);
  assert.equal(isValidEmail("no-at-sign.example.com"), false);
  assert.equal(isValidEmail("missing@tld"), false);
  assert.equal(isValidEmail("spaces in@example.com"), false);
  assert.equal(isValidEmail(""), false);
});

test("signUp resolves for a valid email and strong-enough password", async () => {
  const result = await signUp({
    email: "cynthia@example.com",
    password: "a".repeat(MIN_PASSWORD_LENGTH),
  });
  assert.deepEqual(result, { ok: true });
});

test("signUp rejects an invalid email with a stable error code", async () => {
  await assert.rejects(
    signUp({ email: "not-an-email", password: "long-enough-password" }),
    (error) =>
      error instanceof OnboardingAccountError && error.code === "email_invalid",
  );
});

test("signUp rejects a short password with password_weak", async () => {
  await assert.rejects(
    signUp({
      email: "cynthia@example.com",
      password: "a".repeat(MIN_PASSWORD_LENGTH - 1),
    }),
    (error) =>
      error instanceof OnboardingAccountError && error.code === "password_weak",
  );
});

test("logIn resolves for a plausible credential shape", async () => {
  assert.deepEqual(
    await logIn({ email: "cynthia@example.com", password: "hunter22" }),
    { ok: true },
  );
});

test("logIn rejects bad shapes with credentials_invalid", async () => {
  for (const attempt of [
    { email: "not-an-email", password: "hunter22" },
    { email: "cynthia@example.com", password: "" },
  ]) {
    await assert.rejects(
      logIn(attempt),
      (error) =>
        error instanceof OnboardingAccountError &&
        error.code === "credentials_invalid",
    );
  }
});

test("listInvitedCommunities returns fixture invites with the fields the hub renders", async () => {
  const invites = await listInvitedCommunities();
  assert.ok(invites.length >= 1);
  for (const invite of invites) {
    assert.equal(typeof invite.inviteId, "string");
    assert.equal(typeof invite.name, "string");
    assert.equal(typeof invite.host, "string");
    assert.match(invite.relayWsUrl, /^wss?:\/\//);
  }
});

test("connectInvitedCommunity resolves a known invite and rejects an unknown one", async () => {
  const [invite] = await listInvitedCommunities();
  const result = await connectInvitedCommunity(invite.inviteId);
  assert.equal(result.relayWsUrl, invite.relayWsUrl);

  await assert.rejects(
    connectInvitedCommunity("definitely-not-an-invite"),
    (error) => error instanceof OnboardingAccountError,
  );
});

test("createCommunity slugs the relay host from the name", async () => {
  const created = await createCommunity({
    name: "The Land of Ooo",
    description: "A magical workspace",
  });
  assert.equal(created.name, "The Land of Ooo");
  assert.equal(
    created.relayWsUrl,
    "wss://the-land-of-ooo.communities.buzz.xyz",
  );
});

test("createCommunity rejects an empty name", async () => {
  await assert.rejects(
    createCommunity({ name: "   ", description: "" }),
    (error) => error instanceof OnboardingAccountError,
  );
});

test("sendCommunityInvites counts only valid emails", async () => {
  const result = await sendCommunityInvites({
    emails: ["kalvin@example.com", "not-an-email", "wes@example.com"],
  });
  assert.deepEqual(result, { sent: 2 });
});
