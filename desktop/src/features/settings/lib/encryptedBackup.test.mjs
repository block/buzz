import assert from "node:assert/strict";
import test from "node:test";
import {
  MIN_PASSPHRASE_LEN,
  downloadDisabled,
  isEncrypting,
  passphraseIssue,
  pendingEncryptPassphrase,
  effectivePassphrase,
  encryptedBackupReducer,
  initialEncryptedBackupState,
} from "./encryptedBackup.ts";
const reduce = (events, from = initialEncryptedBackupState) =>
  events.reduce(encryptedBackupReducer, from);
test("password validation mirrors Rust character counting", () => {
  assert.equal(passphraseIssue(""), null);
  assert.match(passphraseIssue("short"), new RegExp(`${MIN_PASSPHRASE_LEN}`));
  const emoji = "😀".repeat(MIN_PASSPHRASE_LEN);
  assert.equal(passphraseIssue(emoji), null);
  assert.equal(
    effectivePassphrase(reduce([{ type: "set-passphrase", value: emoji }])),
    emoji,
  );
});
test("valid password requests encryption without copying it into events", () => {
  const ready = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
  ]);
  assert.equal(pendingEncryptPassphrase(ready), "one-two-three-four");
  const started = reduce([{ type: "encrypt-started", requestId: 1 }], ready);
  assert.equal(isEncrypting(started), true);
  assert.equal(started.requestId, 1);
  assert.equal(Object.hasOwn(started, "encryptingPassphrase"), false);
});
test("background success retains the password without committing the download", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "encrypt-succeeded", requestId: 1, ncryptsec: "ncryptsec1abc" },
  ]);
  assert.equal(state.passphrase, "one-two-three-four");
  assert.equal(state.encrypted, "ncryptsec1abc");
  assert.equal(state.ncryptsec, null);
  assert.equal(state.savedPassword, false);
  assert.equal(state.requestId, null);
  assert.equal(downloadDisabled(state), false);
});
test("submit commits a completed preload immediately", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "encrypt-succeeded", requestId: 1, ncryptsec: "ncryptsec1abc" },
    { type: "download-clicked" },
  ]);
  assert.equal(state.ncryptsec, "ncryptsec1abc");
  assert.equal(state.passphrase, "");
  assert.equal(state.savedPassword, true);
});
test("stale async completions cannot replace current request", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "set-passphrase", value: "five-six-seven-eight" },
    { type: "encrypt-started", requestId: 2 },
    { type: "encrypt-succeeded", requestId: 1, ncryptsec: "ncryptsec1stale" },
  ]);
  assert.equal(state.requestId, 2);
  assert.equal(state.encrypted, null);
  assert.equal(state.passphrase, "five-six-seven-eight");
});
test("failure clears submitted password", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "download-clicked" },
    { type: "encrypt-failed", requestId: 1, message: "keychain unavailable" },
  ]);
  assert.equal(state.passphrase, "");
  assert.equal(state.createError, "keychain unavailable");
  assert.equal(state.downloadPending, false);
  assert.equal(downloadDisabled(state), true);
});
test("background failure stays silent until submit retries encryption", () => {
  const failed = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "encrypt-failed", requestId: 1, message: "keychain unavailable" },
  ]);
  assert.equal(failed.passphrase, "one-two-three-four");
  assert.equal(failed.createError, "keychain unavailable");
  assert.equal(pendingEncryptPassphrase(failed), null);

  const retrying = reduce([{ type: "download-clicked" }], failed);
  assert.equal(retrying.createError, null);
  assert.equal(retrying.downloadPending, true);
  assert.equal(pendingEncryptPassphrase(retrying), "one-two-three-four");
});
test("queued download commits and clears password", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", requestId: 1 },
    { type: "download-clicked" },
    { type: "encrypt-succeeded", requestId: 1, ncryptsec: "ncryptsec1abc" },
  ]);
  assert.equal(state.ncryptsec, "ncryptsec1abc");
  assert.equal(state.passphrase, "");
  assert.equal(state.savedPassword, true);
});
test("starting over discards blob and invalidates late requests", () => {
  const made = {
    ...initialEncryptedBackupState,
    ncryptsec: "ncryptsec1abc",
    encrypted: "ncryptsec1abc",
    savedPassword: true,
    nextRequestId: 3,
  };
  const fresh = reduce([{ type: "start-new-backup" }], made);
  assert.equal(fresh.ncryptsec, null);
  assert.equal(fresh.nextRequestId, 4);
  assert.equal(
    reduce(
      [
        {
          type: "encrypt-succeeded",
          requestId: 2,
          ncryptsec: "ncryptsec1stale",
        },
      ],
      fresh,
    ).ncryptsec,
    null,
  );
});
