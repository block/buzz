import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import {
  claimDraftSend,
  clearAllDrafts,
  deleteDraftEntry,
  getDraftAuthority,
  initDraftStore,
  loadDraftEntry,
  persistDraftEntry,
  recordDraftAuthoredContent,
  saveDraftEntry,
} from "./useDrafts.ts";

beforeEach(() => {
  const storage = new Map();
  globalThis.localStorage = {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
    removeItem: (key) => storage.delete(key),
  };
  clearAllDrafts();
  initDraftStore("author", "wss://authority.example");
});

test("shared authority remains readable when authored empty removes the record", () => {
  const oldVisit = getDraftAuthority("A");
  persistDraftEntry("A", "original", "channel", [], []);
  const revision = oldVisit.revision;
  persistDraftEntry("A", "", "channel", [], []); // optimistic clear
  assert.equal(oldVisit.revision, revision);
  assert.equal(oldVisit.emptyContentIsAuthoritative, false);
  assert.equal(loadDraftEntry("A"), undefined);
  const newVisit = getDraftAuthority("A");
  assert.equal(oldVisit, newVisit);
  recordDraftAuthoredContent("A", "new text");
  recordDraftAuthoredContent("A", "");
  assert.notEqual(oldVisit.revision, revision);
  assert.equal(oldVisit.emptyContentIsAuthoritative, true);
  assert.equal(loadDraftEntry("A"), undefined);
  const afterClear = oldVisit.revision;
  recordDraftAuthoredContent("B", "other draft");
  assert.equal(oldVisit.revision, afterClear);
});

test("explicit deletion supersedes an old attempt even if the value is absent", () => {
  const authority = getDraftAuthority("A");
  claimDraftSend("A");
  const revision = authority.revision;
  deleteDraftEntry("A");
  assert.equal(loadDraftEntry("A"), undefined);
  assert.notEqual(authority.revision, revision);
  assert.equal(authority.emptyContentIsAuthoritative, true);
});

test("new sends revoke recovery without manufacturing authored emptiness", () => {
  const authority = getDraftAuthority("A");
  claimDraftSend("A");
  const first = authority.revision;
  claimDraftSend("A");
  assert.notEqual(authority.revision, first);
  assert.equal(authority.emptyContentIsAuthoritative, false);
});

test("scope reset invalidates retained handles, including a round trip", () => {
  const old = getDraftAuthority("A");
  recordDraftAuthoredContent("A", "");
  const revision = old.revision;
  initDraftStore("other", "wss://other.example");
  assert.notEqual(old.revision, revision);
  assert.equal(getDraftAuthority("A").emptyContentIsAuthoritative, false);
  initDraftStore("author", "wss://authority.example");
  assert.notEqual(getDraftAuthority("A"), old);
  assert.notEqual(old.revision, revision);
});

test("explicit replacement revokes old authority even for an identical snapshot", () => {
  persistDraftEntry("A", "same", "channel", [], []);
  const value = loadDraftEntry("A");
  const authority = getDraftAuthority("A");
  const revision = authority.revision;
  saveDraftEntry("A", value);
  assert.notEqual(authority.revision, revision);
});
