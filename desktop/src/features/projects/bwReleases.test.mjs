import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { bwEvidenceReasonLabel, bwReleasesView } from "./bwReleases.ts";

// P5_6A: the read-only release/run/artifact views render Core-authored
// snapshots only. Every snapshot here is the exact `bw_projection::snapshot`
// output for a pinned `docs/nips/NIP-BW.fixtures.json` case, exported through
// the real Core consumer (crates/buzz-core/tests/bw_desktop_bridge.rs,
// `export_release_view_snapshots`). These tests never invent projection
// state; they assert the view layer reads what Core decided.
const golden = JSON.parse(
  readFileSync(
    new URL("./fixtures/bw-release-snapshots.json", import.meta.url),
    "utf8",
  ),
);
const snap = (name) => {
  const snapshot = golden.snapshots[name];
  assert.ok(snapshot, `missing golden snapshot ${name}`);
  return snapshot;
};

const FIXTURE_SHA256 =
  "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a";
const SET_ID =
  "7e6f1cd96e26672db381e3e614cbfe2b869fdadb020dc7194f94ec886d80004b";
const PIPELINE_ID =
  "a601a11d0a3837fd45e70ad5f8de88cff4f87c7b6b47f4df77efc16d6868f35a";
const REQUEST_ID =
  "488b722ab879ab32101cbad74ae597668565858d69131a6fdc6ffb01fa8a050e";
const RETRY_REQUEST_ID =
  "14532daf1c99c6d2dec5c897fe4a8418ea3a27b20044bad70b8c92fe9128fa8e";
const RUN_ID =
  "78e9c8627b70374113b8f7bfc3bde1febb74c34770c0da8739d943caeba7524a";
const FAILED_RUN_ID =
  "7cf036e4a4a45b9865920c23194efb8f794d89b1d8c02b4c4a04a5c6756f0024";
const RETRY_RUN_ID =
  "27a81fd943174ebb0fdcddc3ea444452518a5525743fdf27941c9802d3ee2cce";
const ARTIFACT_ID =
  "8e47461cc2614ead6e0a52278a3c92ade6bcd114fd8a332c40f2d5546e29d308";
const ARTIFACT_SHA256 =
  "cbc6959eeaf81047a897b88e4d1a965a4bab4e62bfcaabc9ad49f287347c9976";
const NEW_ARTIFACT_ID =
  "61475b3ebce622134d98fe7a7de15320854b7208d3ad430fd27b2d3565a94a87";
const DIGEST_MISMATCH_ARTIFACT_ID =
  "bd25fc24f3fbe9691d1574ab1970bfc9212dbff4e134dddc13729ed32b9983dc";
const HANDOFF_ID =
  "6f6619677ff5185bbc4a781dfa46fd8441cc99157bfa6e43819cec559fde458a";
const LATE_HANDOFF_ID =
  "f5769bd40dbc693a84c17365bfb4136688da1d3f6e48bcae6bc6521c2c3d62a9";
const NEW_ARTIFACT_HANDOFF_ID =
  "6866b0d30ea7a97b4e4fc777602f31f29a1391b167e4a02272916a5e01220c88";
const ISSUE_A =
  "e62948cd5ffabf13a2e9076ac236564bdb201f853d8fa89d2576999156bc78bc";
const ISSUE_B =
  "7b06c85c626cf41862412f217079ae4eeaeb217d633dfd0f198b044db7fef9c1";
const FREEZE_SHA = "4444444444444444444444444444444444444444";
const LATE_HANDOFF_TESTER =
  "2f01e5e15cca351daff3843fb70f3c2f0a1bdd05e5af888a67784ef3e10a2a01";

test("golden snapshots stay bound to the pinned fixture corpus", () => {
  assert.equal(golden.format, "bw-release-snapshots-v1");
  assert.equal(golden.source, "docs/nips/NIP-BW.fixtures.json");
  assert.equal(golden.fixture_sha256, FIXTURE_SHA256);
});

test("a history without any release record renders as an honest empty view", () => {
  const view = bwReleasesView(snap("issue-state-positive"));
  assert.deepEqual(view.sets, []);
  assert.deepEqual(view.incomplete, []);
  assert.deepEqual(view.conflicts, []);
  assert.equal(view.building, false);
  assert.equal(view.dispatchCount, 0);
});

test("set details, members, freeze SHA, pipeline, request and run come from the accepted projection", () => {
  const view = bwReleasesView(snap("build-run-positive"));
  assert.equal(view.sets.length, 1);
  const set = view.sets[0];
  assert.equal(set.id, SET_ID);
  assert.equal(set.release, "pilot-1");
  assert.equal(set.platform, "windows");
  assert.equal(set.stream, "windows-integration");
  assert.equal(set.freezeSha, FREEZE_SHA);
  assert.equal(set.pipeline?.id, PIPELINE_ID);
  assert.equal(
    set.pipeline?.workflow,
    ".github/workflows/windows-fork-integration.yml",
  );
  assert.equal(set.pipeline?.provider, "fixture-ci");
  assert.deepEqual(
    set.members.map((member) => member.issue),
    [ISSUE_A, ISSUE_B],
  );
  assert.equal(set.members[0].state, "implemented");
  assert.equal(set.requests.length, 1);
  assert.equal(set.requests[0].id, REQUEST_ID);
  assert.equal(set.requests[0].attempt, 1);
  assert.equal(set.runs.length, 1);
  assert.equal(set.runs[0].id, RUN_ID);
  assert.equal(set.runs[0].result, "success");
  assert.equal(set.runs[0].request, REQUEST_ID);
  assert.equal(set.runs[0].provider, "fixture-ci");
  assert.equal(set.runs[0].runId, "run-1");
  assert.deepEqual(set.artifacts, []);
  assert.equal(set.handoff, null);
});

test("provider run success alone never turns the set or its verdicts completed", () => {
  const view = bwReleasesView(snap("build-run-positive"));
  const set = view.sets[0];
  // The set status is Core's `projection.sets` value verbatim — the view
  // layer never upgrades it from a successful run.
  assert.equal(set.status, "active");
  assert.equal(set.runs[0].result, "success");
  assert.deepEqual(view.incomplete, []);
  assert.equal(view.building, false);
});

test("a failed run and its retry render as distinct runs, ordered by attempt", () => {
  const view = bwReleasesView(snap("retry-success"));
  const set = view.sets[0];
  assert.equal(set.status, "active");
  assert.deepEqual(
    set.requests.map((request) => request.attempt),
    [1, 2],
  );
  assert.equal(set.requests[1].id, RETRY_REQUEST_ID);
  assert.equal(set.requests[1].retryOf, FAILED_RUN_ID);
  assert.deepEqual(
    set.runs.map((run) => [run.attempt, run.result]),
    [
      [1, "failure"],
      [2, "success"],
    ],
  );
  assert.equal(set.runs[0].id, FAILED_RUN_ID);
  assert.equal(set.runs[1].id, RETRY_RUN_ID);
  assert.equal(view.dispatchCount, 2);
});

test("production-shaped history without external evidence stays visibly pending", () => {
  const view = bwReleasesView(snap("build-run-positive-production-shape"));
  // No accepted release-set: the view must not materialize a set from
  // signed-but-unverified events.
  assert.deepEqual(view.sets, []);
  assert.equal(view.hostAuthorized, false);
  assert.equal(view.dispatchCount, 0);
  const pending = Object.fromEntries(
    view.incomplete.map((entry) => [entry.eventId, entry]),
  );
  assert.equal(pending[SET_ID]?.outcome, "pending");
  assert.equal(pending[SET_ID]?.stage, "external");
  assert.equal(pending[SET_ID]?.code, "relay-head");
  assert.equal(pending[REQUEST_ID]?.outcome, "pending");
  assert.equal(pending[RUN_ID]?.outcome, "pending");
  assert.match(bwEvidenceReasonLabel(pending[RUN_ID]), /git head observation/i);
});

test("a download digest mismatch is refused visibly, never rendered as an artifact", () => {
  const view = bwReleasesView(snap("artifact-digest"));
  const set = view.sets[0];
  assert.equal(set.status, "active");
  assert.deepEqual(set.artifacts, []);
  const refused = view.incomplete.find(
    (entry) => entry.eventId === DIGEST_MISMATCH_ARTIFACT_ID,
  );
  assert.equal(refused?.outcome, "reject");
  assert.equal(refused?.stage, "external");
  assert.equal(refused?.code, "artifact-digest");
  assert.match(bwEvidenceReasonLabel(refused), /digest mismatch/i);
});

test("an ephemeral download refuses the handoff while the artifact itself stays listed", () => {
  const view = bwReleasesView(snap("ephemeral-download"));
  const set = view.sets[0];
  assert.equal(set.handoff, null);
  assert.equal(set.artifacts.length, 1);
  const artifact = set.artifacts[0];
  assert.equal(artifact.id, ARTIFACT_ID);
  assert.equal(artifact.sha256, ARTIFACT_SHA256);
  assert.equal(artifact.bytes, 37);
  assert.equal(artifact.mime, "application/vnd.microsoft.portable-executable");
  assert.equal(
    artifact.url,
    `https://download.example.invalid/immutable/${ARTIFACT_SHA256}/pilot.exe`,
  );
  assert.deepEqual(artifact.verdicts, {
    [ISSUE_A]: "unreviewed",
    [ISSUE_B]: "unreviewed",
  });
  const refused = view.incomplete.find((entry) => entry.eventId === HANDOFF_ID);
  assert.equal(refused?.outcome, "reject");
  assert.equal(refused?.code, "durability");
  assert.match(bwEvidenceReasonLabel(refused), /not durable/i);
});

test("a late-delivered handoff shows the Core-projected current handoff, not a timestamp guess", () => {
  const view = bwReleasesView(snap("late-handoff-delivery"));
  const set = view.sets[0];
  // Core's projection.handoffs head — the later delivery — wins regardless
  // of arrival order; the accepted late verdict is visible per member.
  assert.equal(set.handoff?.id, LATE_HANDOFF_ID);
  assert.equal(set.handoff?.tester, LATE_HANDOFF_TESTER);
  assert.equal(set.handoff?.installation?.unsigned, true);
  assert.equal(set.handoff?.installation?.immutable, true);
  assert.equal(set.artifacts.length, 1);
  assert.deepEqual(set.artifacts[0].verdicts, {
    [ISSUE_A]: "accepted",
    [ISSUE_B]: "unreviewed",
  });
  assert.deepEqual(view.incomplete, []);
});

test("a historically accepted issue keeps its resolution while a new artifact starts unreviewed", () => {
  const view = bwReleasesView(snap("new-artifact-no-inherited-review"));
  const set = view.sets[0];
  // Historical acceptance: both members resolved and the set completed by
  // Core — but only for the reviewed artifact.
  assert.equal(set.status, "completed");
  assert.deepEqual(
    set.members.map((member) => member.state),
    ["resolved", "resolved"],
  );
  const reviewed = set.artifacts.find(
    (artifact) => artifact.id === ARTIFACT_ID,
  );
  const fresh = set.artifacts.find(
    (artifact) => artifact.id === NEW_ARTIFACT_ID,
  );
  assert.deepEqual(reviewed?.verdicts, {
    [ISSUE_A]: "accepted",
    [ISSUE_B]: "accepted",
  });
  // The new artifact inherits nothing: unreviewed despite the resolved issues.
  assert.deepEqual(fresh?.verdicts, {
    [ISSUE_A]: "unreviewed",
    [ISSUE_B]: "unreviewed",
  });
  assert.equal(set.handoff?.id, NEW_ARTIFACT_HANDOFF_ID);
  assert.deepEqual(set.handoff?.artifacts, [NEW_ARTIFACT_ID]);
});

test("conflicting member verdicts surface as a conflict with both references", () => {
  const view = bwReleasesView(snap("verdict-conflict"));
  const set = view.sets[0];
  assert.deepEqual(set.artifacts[0].verdicts, {
    [ISSUE_A]: "conflict",
    [ISSUE_B]: "unreviewed",
  });
  assert.equal(view.conflicts.length, 2);
  const conflicts = view.incomplete.filter(
    (entry) => entry.code === "verdict-conflict",
  );
  assert.equal(conflicts.length, 2);
  assert.ok(conflicts.every((entry) => entry.outcome === "conflict"));
  assert.ok(
    view.conflicts.every((id) => conflicts.some((c) => c.eventId === id)),
  );
});

test("unknown evidence codes fall back to an honest stage/code label", () => {
  assert.equal(
    bwEvidenceReasonLabel({
      eventId: "x",
      outcome: "pending",
      stage: "external",
      code: "some-future-code",
    }),
    "external: some-future-code",
  );
});
