import { readFileSync } from "node:fs";

import { expect, test } from "@playwright/test";

import type { MockBwSnapshotOverride } from "../../src/testing/e2eBridge";
import { installMockBridge } from "../helpers/bridge";

// P5_6A: read-only release/run/artifact views. Every snapshot fed through the
// existing transient `get_project_bw` mock seam is the exact Core
// `bw_projection::snapshot` output for a pinned `docs/nips/NIP-BW.fixtures.json`
// case (SHA-256 b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a),
// exported through the real Core consumer — see
// `crates/buzz-core/tests/bw_desktop_bridge.rs::export_release_view_snapshots`.
// These specs prove the desktop renders those Core states faithfully: no
// invented evidence, no completed-status from a provider success, missing
// external evidence stays visibly pending, and no side-effecting control
// exists in the panel.

type BwGoldenSnapshot = MockBwSnapshotOverride & { repo: string };

const golden = JSON.parse(
  readFileSync(
    new URL(
      "../../src/features/projects/fixtures/bw-release-snapshots.json",
      import.meta.url,
    ),
    "utf8",
  ),
) as {
  format: string;
  source: string;
  fixture_sha256: string;
  snapshots: Record<string, BwGoldenSnapshot>;
};

const snapshots = golden.snapshots;

function bwSnapshotFor(name: string): MockBwSnapshotOverride {
  const snapshot = snapshots[name];
  if (!snapshot) throw new Error(`missing golden snapshot ${name}`);
  return {
    activation: snapshot.activation ?? null,
    records: snapshot.records,
    decisions: snapshot.decisions,
    notices: snapshot.notices,
    projection: snapshot.projection,
  };
}

const SET_ID =
  "7e6f1cd96e26672db381e3e614cbfe2b869fdadb020dc7194f94ec886d80004b";
const RUN_ID =
  "78e9c8627b70374113b8f7bfc3bde1febb74c34770c0da8739d943caeba7524a";
const FREEZE_SHA = "4444444444444444444444444444444444444444";
const ARTIFACT_SHA256 =
  "cbc6959eeaf81047a897b88e4d1a965a4bab4e62bfcaabc9ad49f287347c9976";
const NEW_ARTIFACT_ID =
  "61475b3ebce622134d98fe7a7de15320854b7208d3ad430fd27b2d3565a94a87";
const DIGEST_MISMATCH_ARTIFACT_ID =
  "bd25fc24f3fbe9691d1574ab1970bfc9212dbff4e134dddc13729ed32b9983dc";
const CONFLICT_VERDICT_A =
  "321546fb192b27f53c8717bb781db0e6eb5f43bb49b01f72d896470531ef4a45";
const CONFLICT_VERDICT_B =
  "4b23635e00a49c5a3337fecca0698365e4a6b1a1353f70eacb843180b81ca08a";

// The projects surface is a preview feature — opt in before the app mounts.
async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

async function openReleasesTab(
  page: import("@playwright/test").Page,
  goldenCase: string,
) {
  await enableProjectsFeature(page);
  await installMockBridge(page, { bwSnapshot: bwSnapshotFor(goldenCase) });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-repositories").click();
  await page.getByRole("button", { name: "View buzz" }).click();
  await page.getByRole("tab", { name: "Releases" }).click();
  const panel = page.getByTestId("bw-releases-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return panel;
}

test("golden snapshots stay bound to the pinned fixture corpus", () => {
  expect(golden.format).toBe("bw-release-snapshots-v1");
  expect(golden.source).toBe("docs/nips/NIP-BW.fixtures.json");
  expect(golden.fixture_sha256).toBe(
    "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a",
  );
});

test("an accepted run renders the frozen set without ever claiming completed", async ({
  page,
}) => {
  const panel = await openReleasesTab(page, "build-run-positive");

  const set = panel.getByTestId("bw-release-set");
  await expect(set).toBeVisible();
  await expect(set).toContainText("pilot-1");
  await expect(set.getByTestId("bw-release-set-status")).toHaveText("active");
  await expect(set.getByTestId("bw-release-freeze-sha")).toContainText(
    FREEZE_SHA,
  );
  await expect(set.getByTestId("bw-release-pipeline")).toContainText(
    ".github/workflows/windows-fork-integration.yml",
  );
  await expect(set.getByTestId("bw-release-member")).toHaveCount(2);
  const run = set.getByTestId("bw-release-run");
  await expect(run).toHaveCount(1);
  await expect(run.getByTestId("bw-release-run-result")).toHaveText("success");
  // Provider success alone: no artifact, no handoff, no completed set, and no
  // invented evidence anywhere in the panel.
  await expect(set.getByTestId("bw-release-no-artifact")).toBeVisible();
  await expect(set.getByTestId("bw-release-no-handoff")).toBeVisible();
  await expect(panel.getByTestId("bw-release-incomplete")).toHaveCount(0);
  await expect(panel).not.toContainText("completed");
  // Read-only slice: the panel exposes no actionable control.
  await expect(panel.locator("button")).toHaveCount(0);
});

test("production-shaped history without external evidence stays visibly pending", async ({
  page,
}) => {
  const panel = await openReleasesTab(
    page,
    "build-run-positive-production-shape",
  );

  // Nothing was accepted into a set: the view must not materialize one from
  // signed-but-unverified events.
  await expect(panel.getByTestId("bw-release-set")).toHaveCount(0);
  await expect(panel.getByTestId("bw-releases-empty")).toContainText(
    "No release set has been frozen yet",
  );
  const incomplete = panel.getByTestId("bw-release-incomplete");
  await expect(incomplete).toBeVisible();
  const entries = incomplete.getByTestId("bw-release-incomplete-entry");
  await expect(entries).not.toHaveCount(0);
  // The frozen set and its run stay pending on the missing Git observation,
  // as traceable event references.
  await expect(incomplete).toContainText(SET_ID.slice(0, 12));
  await expect(incomplete).toContainText(RUN_ID.slice(0, 12));
  await expect(incomplete).toContainText(
    "Awaiting canonical Git head observation",
  );
});

test("a digest-mismatched download is refused visibly, never rendered as an artifact", async ({
  page,
}) => {
  const panel = await openReleasesTab(page, "artifact-digest");

  const set = panel.getByTestId("bw-release-set");
  await expect(set.getByTestId("bw-release-set-status")).toHaveText("active");
  await expect(set.getByTestId("bw-release-artifact")).toHaveCount(0);
  await expect(set.getByTestId("bw-release-no-artifact")).toBeVisible();
  const incomplete = panel.getByTestId("bw-release-incomplete");
  await expect(incomplete).toContainText(
    DIGEST_MISMATCH_ARTIFACT_ID.slice(0, 12),
  );
  await expect(incomplete).toContainText("Download digest mismatch");
});

test("historical acceptance stays separate from a new unreviewed artifact", async ({
  page,
}) => {
  const panel = await openReleasesTab(page, "new-artifact-no-inherited-review");

  const set = panel.getByTestId("bw-release-set");
  // Core completed the set on the reviewed artifact; both issues resolved.
  await expect(set.getByTestId("bw-release-set-status")).toHaveText(
    "completed",
  );
  await expect(set.getByTestId("bw-release-member-state")).toHaveCount(2);
  for (const state of await set.getByTestId("bw-release-member-state").all()) {
    await expect(state).toHaveText("resolved");
  }
  const artifacts = set.getByTestId("bw-release-artifact");
  await expect(artifacts).toHaveCount(2);
  // The new artifact inherits nothing: both member verdicts unreviewed while
  // the issues remain historically resolved.
  const fresh = artifacts.filter({
    has: page.locator(`text=${NEW_ARTIFACT_ID.slice(0, 12)}`),
  });
  await expect(fresh).toHaveCount(1);
  await expect(fresh.getByTestId("bw-release-artifact-verdict")).toHaveCount(2);
  for (const verdict of await fresh
    .getByTestId("bw-release-artifact-verdict")
    .all()) {
    await expect(verdict).toContainText("unreviewed");
  }
  const reviewed = artifacts.filter({
    has: page.locator(`text=${ARTIFACT_SHA256}`),
  });
  await expect(reviewed).toHaveCount(1);
  for (const verdict of await reviewed
    .getByTestId("bw-release-artifact-verdict")
    .all()) {
    await expect(verdict).toContainText("accepted");
  }
  // Explicit artifact facts and the unsigned/immutable handoff warnings.
  await expect(reviewed.getByTestId("bw-release-artifact-sha256")).toHaveText(
    ARTIFACT_SHA256,
  );
  const handoff = set.getByTestId("bw-release-handoff");
  await expect(handoff).toBeVisible();
  await expect(
    handoff.getByTestId("bw-release-handoff-unsigned"),
  ).toContainText("Unsigned installer");
  await expect(handoff.getByTestId("bw-release-handoff-immutable")).toHaveText(
    "immutable",
  );
  await expect(handoff.getByTestId("bw-release-handoff-tester")).toBeVisible();
});

test("conflicting member verdicts surface as a conflict with both references", async ({
  page,
}) => {
  const panel = await openReleasesTab(page, "verdict-conflict");

  const set = panel.getByTestId("bw-release-set");
  await expect(set.getByTestId("bw-release-set-status")).toHaveText("active");
  await expect(set).toContainText("conflict");
  const incomplete = panel.getByTestId("bw-release-incomplete");
  await expect(incomplete).toContainText(CONFLICT_VERDICT_A.slice(0, 12));
  await expect(incomplete).toContainText(CONFLICT_VERDICT_B.slice(0, 12));
  await expect(incomplete).toContainText("Conflicting member verdicts");
});
