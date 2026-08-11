import assert from "node:assert/strict";
import test from "node:test";

import {
  clearIdentityHandoffVault,
  destroyIdentityHandoff,
  getIdentityHandoff,
  identityHandoffVaultSize,
  setIdentityHandoffPolicyReceipt,
  storeIdentityHandoff,
} from "./identityHandoffVault.ts";

const CODE = `v3.${"a".repeat(64)}`;

test.beforeEach(() => clearIdentityHandoffVault());

test("identity handoff secrets live only under non-secret transaction ids", () => {
  storeIdentityHandoff("transaction-1", {
    code: CODE,
    policyReceipt: "policy-receipt",
  });

  assert.deepEqual(getIdentityHandoff("transaction-1"), {
    code: CODE,
    policyReceipt: "policy-receipt",
  });
  assert.equal(identityHandoffVaultSize(), 1);
});

test("destroy and process restart abandon live handoff credentials", () => {
  storeIdentityHandoff("transaction-1", { code: CODE });
  destroyIdentityHandoff("transaction-1");
  assert.equal(getIdentityHandoff("transaction-1"), null);

  storeIdentityHandoff("transaction-2", { code: CODE });
  clearIdentityHandoffVault();
  assert.equal(getIdentityHandoff("transaction-2"), null);
  assert.equal(identityHandoffVaultSize(), 0);
});

test("policy acceptance adds a receipt without replacing the live code", () => {
  storeIdentityHandoff("transaction-1", { code: CODE });

  assert.equal(
    setIdentityHandoffPolicyReceipt("transaction-1", "fresh-policy-receipt"),
    true,
  );
  assert.deepEqual(getIdentityHandoff("transaction-1"), {
    code: CODE,
    policyReceipt: "fresh-policy-receipt",
  });
  assert.equal(
    setIdentityHandoffPolicyReceipt("missing-transaction", "receipt"),
    false,
  );
});
