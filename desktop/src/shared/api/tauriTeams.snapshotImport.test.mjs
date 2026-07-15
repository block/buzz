import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveImportPhase,
  getProfileSyncFailures,
  deriveImportToast,
  buildTeamSnapshotSendArgs,
} from "../../features/agents/ui/teamSnapshotImport.lib.ts";

// Behavior tests for the team snapshot import dialog flow and the send-encode
// contract. These import and exercise the actual production functions used by
// TeamSnapshotImportDialog, useTeamActions, and TeamSnapshotSendDialog.

// ── Factories ────────────────────────────────────────────────────────────────

function makeMemberResult(overrides = {}) {
  return {
    displayName: "Agent A",
    pubkey: "aabb".repeat(16),
    personaId: "persona-uuid-1",
    memoryWritten: 0,
    memoryTotal: 0,
    memoryErrors: [],
    profileSyncError: null,
    ...overrides,
  };
}

function makeImportResult(overrides = {}) {
  const { members: memberOverrides, team: teamOverrides, ...rest } = overrides;
  return {
    team: {
      id: "team-uuid",
      name: "Test Team",
      description: "A test team",
      personaIds: ["persona-uuid-1"],
      instructions: "Be helpful",
      isBuiltin: false,
      sourceDir: null,
      isSymlink: false,
      symlinkTarget: null,
      version: null,
      createdAt: new Date().toISOString(),
      ...teamOverrides,
    },
    personaIds: ["persona-uuid-1"],
    members: [makeMemberResult()],
    ...rest,
    ...(memberOverrides !== undefined ? { members: memberOverrides } : {}),
  };
}

// ── deriveImportPhase ────────────────────────────────────────────────────────

test("deriveImportPhase_returns_result_when_result_present", () => {
  const result = makeImportResult();
  assert.equal(deriveImportPhase(result, false), "result");
});

test("deriveImportPhase_returns_result_even_when_confirming", () => {
  // result takes precedence over isConfirming
  const result = makeImportResult();
  assert.equal(deriveImportPhase(result, true), "result");
});

test("deriveImportPhase_returns_confirming_when_inflight", () => {
  assert.equal(deriveImportPhase(null, true), "confirming");
});

test("deriveImportPhase_returns_preview_when_idle", () => {
  assert.equal(deriveImportPhase(null, false), "preview");
});

test("deriveImportPhase_confirm_failure_returns_to_preview", () => {
  // When confirm throws, result stays null and isConfirming resets to false
  assert.equal(deriveImportPhase(null, false), "preview");
});

// ── getProfileSyncFailures ───────────────────────────────────────────────────

test("getProfileSyncFailures_returns_members_with_errors", () => {
  const members = [
    makeMemberResult({ displayName: "A", profileSyncError: "relay timeout" }),
    makeMemberResult({
      displayName: "B",
      pubkey: "ccdd".repeat(16),
      profileSyncError: null,
    }),
  ];
  const failures = getProfileSyncFailures(members);
  assert.equal(failures.length, 1);
  assert.equal(failures[0].displayName, "A");
  assert.equal(failures[0].profileSyncError, "relay timeout");
});

test("getProfileSyncFailures_returns_empty_when_all_ok", () => {
  const members = [
    makeMemberResult({ profileSyncError: null }),
    makeMemberResult({
      displayName: "B",
      pubkey: "ccdd".repeat(16),
      profileSyncError: null,
    }),
  ];
  assert.equal(getProfileSyncFailures(members).length, 0);
});

test("getProfileSyncFailures_returns_all_when_all_failed", () => {
  const members = [
    makeMemberResult({ displayName: "A", profileSyncError: "relay rejected" }),
    makeMemberResult({
      displayName: "B",
      pubkey: "ccdd".repeat(16),
      profileSyncError: "network error",
    }),
  ];
  assert.equal(getProfileSyncFailures(members).length, 2);
});

// ── deriveImportToast ────────────────────────────────────────────────────────

test("deriveImportToast_success_when_no_failures", () => {
  const result = makeImportResult();
  const toast = deriveImportToast(result);
  assert.equal(toast.type, "notice");
  assert.match(toast.message, /Imported Test Team with 1 member\./);
});

test("deriveImportToast_error_when_profile_sync_fails", () => {
  const result = makeImportResult({
    members: [makeMemberResult({ profileSyncError: "relay timeout" })],
  });
  const toast = deriveImportToast(result);
  assert.equal(toast.type, "error");
  assert.match(toast.message, /Test Team imported, but/);
  assert.match(toast.message, /failed to sync profile/);
});

test("deriveImportToast_error_when_memory_errors_only", () => {
  const result = makeImportResult({
    members: [
      makeMemberResult({
        memoryTotal: 2,
        memoryWritten: 0,
        memoryErrors: ["err1", "err2"],
        profileSyncError: null,
      }),
    ],
  });
  const toast = deriveImportToast(result);
  assert.equal(toast.type, "error");
  assert.match(toast.message, /2 memory entr/);
  assert.doesNotMatch(toast.message, /sync profile/);
});

test("deriveImportToast_error_composes_both_failure_types", () => {
  const result = makeImportResult({
    members: [
      makeMemberResult({
        memoryTotal: 3,
        memoryWritten: 1,
        memoryErrors: ["slug err 1", "slug err 2"],
        profileSyncError: "relay timeout",
      }),
    ],
  });
  const toast = deriveImportToast(result);
  assert.equal(toast.type, "error");
  assert.match(toast.message, /2 memory entr/);
  assert.match(toast.message, /failed to sync profile/);
  assert.match(toast.message, / and /);
});

test("deriveImportToast_plural_members", () => {
  const result = makeImportResult({
    members: [
      makeMemberResult(),
      makeMemberResult({
        displayName: "Agent B",
        pubkey: "ccdd".repeat(16),
        personaId: "persona-uuid-2",
      }),
    ],
  });
  const toast = deriveImportToast(result);
  assert.equal(toast.type, "notice");
  assert.match(toast.message, /with 2 members\./);
});

test("deriveImportToast_singular_member", () => {
  const result = makeImportResult();
  const toast = deriveImportToast(result);
  assert.match(toast.message, /with 1 member\./);
});

// ── Mixed success/failure across members ─────────────────────────────────────

test("mixed_member_outcomes_memory_errors_and_profile_sync", () => {
  const result = makeImportResult({
    members: [
      makeMemberResult({
        displayName: "Agent A",
        memoryWritten: 3,
        memoryTotal: 3,
        memoryErrors: [],
        profileSyncError: "relay timeout",
      }),
      makeMemberResult({
        displayName: "Agent B",
        pubkey: "ccdd".repeat(16),
        personaId: "persona-uuid-2",
        memoryWritten: 0,
        memoryTotal: 2,
        memoryErrors: ["err1", "err2"],
        profileSyncError: null,
      }),
      makeMemberResult({
        displayName: "Agent C",
        pubkey: "eeff".repeat(16),
        personaId: "persona-uuid-3",
        memoryWritten: 1,
        memoryTotal: 1,
        memoryErrors: [],
        profileSyncError: null,
      }),
    ],
  });

  const failures = getProfileSyncFailures(result.members);
  assert.equal(failures.length, 1);
  assert.equal(failures[0].displayName, "Agent A");

  const toast = deriveImportToast(result);
  assert.equal(toast.type, "error");
  assert.match(toast.message, /2 memory entr/);
  assert.match(toast.message, /1 member failed to sync profile/);
});

// ── buildTeamSnapshotSendArgs ────────────────────────────────────────────────

test("buildTeamSnapshotSendArgs_always_uses_png_format", () => {
  const args = buildTeamSnapshotSendArgs("team-123", "core");
  assert.equal(args.id, "team-123");
  assert.equal(args.memoryLevel, "core");
  assert.equal(args.format, "png");
});

test("buildTeamSnapshotSendArgs_passes_through_memory_level", () => {
  for (const level of ["none", "core", "everything"]) {
    const args = buildTeamSnapshotSendArgs("t", level);
    assert.equal(args.memoryLevel, level);
    assert.equal(args.format, "png", `format must be png for level "${level}"`);
  }
});
