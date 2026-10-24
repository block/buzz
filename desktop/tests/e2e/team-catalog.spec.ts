import { expect, test } from "@playwright/test";
import { hexToBytes } from "@noble/hashes/utils.js";
import { finalizeEvent } from "nostr-tools/pure";

import type { RelayEvent } from "@/shared/api/types";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

type CatalogMember = {
  memberKey: string;
  displayName: string;
  systemPrompt: string;
  model?: string | null;
  runtime?: string | null;
  provider?: string | null;
};

/** A kind:30178 head, shaped exactly as `team_catalog_content` projects it.
 *
 * The catalog read path runs through the shared signature gate (the #4220
 * hardening ported into `catalogRelay.ts`), so a discoverable head must carry
 * a valid signature over its own contents. Sign with the owner's test key
 * rather than stamping a placeholder `sig`, mirroring the persona fixtures in
 * `agents.spec.ts`. The signed `id` is content-derived, so callers read it off
 * the returned event instead of supplying one. */
function createTeamCatalogEvent(input: {
  ownerPubkey: string;
  teamDTag: string;
  name: string;
  description?: string | null;
  instructions?: string | null;
  members: CatalogMember[];
  createdAt?: number;
  shared?: boolean;
}): RelayEvent {
  const ownerPrivateKey = Object.values(TEST_IDENTITIES).find(
    (identity) => identity.pubkey === input.ownerPubkey,
  )?.privateKey;
  if (!ownerPrivateKey) {
    throw new Error(`No test private key for ${input.ownerPubkey}`);
  }
  return finalizeEvent(
    {
      created_at: input.createdAt ?? 1_721_750_400,
      kind: 30178,
      tags: [
        ["d", input.teamDTag],
        ...(input.shared === false ? [] : [["shared", "true"]]),
      ],
      content: JSON.stringify({
        v: 1,
        name: input.name,
        description: input.description ?? null,
        instructions: input.instructions ?? null,
        members: input.members.map((member) => ({
          member_key: member.memberKey,
          display_name: member.displayName,
          system_prompt: member.systemPrompt,
          avatar_url: null,
          runtime: member.runtime ?? null,
          model: member.model ?? null,
          provider: member.provider ?? null,
        })),
      }),
    },
    hexToBytes(ownerPrivateKey),
  );
}

async function gotoAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
}

async function openTeamCatalog(page: import("@playwright/test").Page) {
  await page.getByTestId("new-team-card").click();
  await page.getByTestId("team-catalog-open").click();
  await expect(page.getByTestId("community-catalog-dialog")).toBeVisible();
}

async function openTeamShareDialog(
  page: import("@playwright/test").Page,
  teamName: string,
) {
  await page.getByLabel(`${teamName} team actions`).click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await expect(page.getByTestId("team-share-dialog")).toBeVisible();
}

async function setTeamCatalogAccess(
  page: import("@playwright/test").Page,
  shared: boolean,
) {
  const toggle = page.getByTestId("team-share-catalog-access");
  const isChecked = await toggle.isChecked();
  if (isChecked !== shared) {
    await toggle.click();
  }
}

async function listMockTeams(page: import("@playwright/test").Page) {
  return page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("Mock invoke bridge is not installed.");
    return (await invoke("list_teams")) as Array<{
      name: string;
      persona_ids: string[];
      catalog_source: { owner_pubkey: string; team_d_tag: string } | null;
    }>;
  });
}

const ALICE_TEAM_D_TAG = "alice-review-crew";
const ALICE_TEAM_MEMBERS: CatalogMember[] = [
  {
    memberKey: "reviewer",
    displayName: "Alice’s Reviewer",
    systemPrompt: "Review changes for the whole community.",
  },
  {
    memberKey: "scribe",
    displayName: "Alice’s Scribe",
    systemPrompt: "Write the summary.",
  },
];

test("an unshared kind 30178 head from another member is not offered", async ({
  page,
}) => {
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice’s Private Crew",
        members: ALICE_TEAM_MEMBERS,
        shared: false,
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);

  await expect(page.getByTestId("community-catalog-empty-state")).toBeVisible();
  await expect(
    page.locator('[data-testid^="community-catalog-team-"]'),
  ).toHaveCount(0);
});

test("adding another member's team records its catalog provenance", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice’s Review Crew",
        description: "Two agents that review and summarise.",
        members: ALICE_TEAM_MEMBERS,
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);

  const entry = page.getByTestId(`community-catalog-team-${entryKey}`);
  await expect(entry).toContainText("Alice’s Review Crew");
  await entry.click();

  const detail = page.getByTestId("community-catalog-detail-pane");
  await expect(detail).toContainText("Added by alice");
  await expect(detail).toContainText("2 members");
  await expect(
    page.getByTestId("community-catalog-member-reviewer"),
  ).toContainText("Alice’s Reviewer");
  await expect(
    page.getByTestId("community-catalog-member-scribe"),
  ).toContainText("Alice’s Scribe");

  await page.getByTestId("community-catalog-add-team").click();
  await expect(
    page.getByText("Added Alice’s Review Crew to your teams."),
  ).toBeVisible();

  // The copy carries a fresh local id, so only the stored coordinate links it
  // back to Alice's publication — that link is what stops a second copy.
  const teams = await listMockTeams(page);
  const added = teams.find((team) => team.name === "Alice’s Review Crew");
  expect(added).toMatchObject({
    catalog_source: {
      owner_pubkey: TEST_IDENTITIES.alice.pubkey,
      team_d_tag: ALICE_TEAM_D_TAG,
    },
  });
  expect(added?.persona_ids).toHaveLength(2);

  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();
  const addButton = page.getByTestId("community-catalog-add-team");
  await expect(addButton).toHaveText("Added to my teams");
  await expect(addButton).toBeDisabled();
  expect(
    (await listMockTeams(page)).filter(
      (team) => team.name === "Alice’s Review Crew",
    ),
  ).toHaveLength(1);
});

test("a head that moved while the dialog was open is rejected", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice’s Review Crew",
        members: ALICE_TEAM_MEMBERS,
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  // Republish the coordinate without notifying subscribers: the dialog keeps
  // rendering — and keeps holding — the superseded event id.
  await page.evaluate(
    ({ ownerPubkey, teamDTag }) => {
      const replace = (
        window as Window & {
          __BUZZ_E2E_REPLACE_MOCK_TEAM_CATALOG_HEAD__?: (
            event: unknown,
          ) => void;
        }
      ).__BUZZ_E2E_REPLACE_MOCK_TEAM_CATALOG_HEAD__;
      if (!replace) throw new Error("Team catalog head seam is not installed.");
      replace({
        id: "3".repeat(64),
        pubkey: ownerPubkey,
        created_at: 1_721_760_400,
        kind: 30178,
        tags: [
          ["d", teamDTag],
          ["shared", "true"],
        ],
        content: JSON.stringify({
          v: 1,
          name: "Alice’s Review Crew",
          description: null,
          instructions: null,
          members: [],
        }),
        sig: "2".repeat(128),
      });
    },
    { ownerPubkey: TEST_IDENTITIES.alice.pubkey, teamDTag: ALICE_TEAM_D_TAG },
  );

  await page.getByTestId("community-catalog-add-team").click();
  await expect(
    page.getByText(
      "This team was updated since you opened the catalog. Reopen it and try again.",
    ),
  ).toBeVisible();
  expect(
    (await listMockTeams(page)).filter(
      (team) =>
        team.catalog_source !== null && team.catalog_source !== undefined,
    ),
  ).toHaveLength(0);
});

test("at an equal timestamp the lower-id head is canonical and a superseding head is rejected as stale", async ({
  page,
}) => {
  // Two signed events for the same coordinate at identical created_at. Both
  // carry valid signatures (the read path verifies them), so the relay's
  // tie-break decides: `created_at DESC, id ASC` makes the lower-id event
  // canonical. Because the signed `id` is content-derived, we cannot pin it to
  // a literal — we sign both, sort by id, and derive the expected canonical
  // name from whichever id sorts first.
  const SAME_TIMESTAMP = 1_721_760_000;
  const crew = createTeamCatalogEvent({
    ownerPubkey: TEST_IDENTITIES.alice.pubkey,
    teamDTag: ALICE_TEAM_D_TAG,
    name: "Alice's Review Crew",
    members: ALICE_TEAM_MEMBERS,
    createdAt: SAME_TIMESTAMP,
  });
  const crewV2 = createTeamCatalogEvent({
    ownerPubkey: TEST_IDENTITIES.alice.pubkey,
    teamDTag: ALICE_TEAM_D_TAG,
    name: "Alice's Review Crew v2",
    members: [],
    createdAt: SAME_TIMESTAMP,
  });
  const [lower] = [crew, crewV2].sort((left, right) =>
    left.id.localeCompare(right.id),
  );
  const canonicalName = JSON.parse(lower.content).name as string;

  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, { teamCatalogEvents: [crew, crewV2] });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  // The lower-id event was selected as canonical, so its name is what renders.
  await expect(page.getByTestId("community-catalog-detail-pane")).toContainText(
    canonicalName,
  );

  // Republish the coordinate with a strictly newer head, without notifying
  // subscribers: the dialog keeps holding the superseded id. The add must be
  // rejected as stale.
  await page.evaluate(
    ({ ownerPubkey, teamDTag }) => {
      const replace = (
        window as Window & {
          __BUZZ_E2E_REPLACE_MOCK_TEAM_CATALOG_HEAD__?: (
            event: unknown,
          ) => void;
        }
      ).__BUZZ_E2E_REPLACE_MOCK_TEAM_CATALOG_HEAD__;
      if (!replace) throw new Error("Team catalog head seam is not installed.");
      replace({
        id: "9".repeat(64),
        pubkey: ownerPubkey,
        created_at: 1_721_760_400,
        kind: 30178,
        tags: [
          ["d", teamDTag],
          ["shared", "true"],
        ],
        content: JSON.stringify({
          v: 1,
          name: "Alice's Review Crew v2",
          description: null,
          instructions: null,
          members: [],
        }),
        sig: "2".repeat(128),
      });
    },
    { ownerPubkey: TEST_IDENTITIES.alice.pubkey, teamDTag: ALICE_TEAM_D_TAG },
  );

  await page.getByTestId("community-catalog-add-team").click();
  await expect(
    page.getByText(
      "This team was updated since you opened the catalog. Reopen it and try again.",
    ),
  ).toBeVisible();
});

test("sharing a team publishes it to the catalog and unsharing retracts it", async ({
  page,
}) => {
  // The own head is published through the same signature-verified read path as
  // foreign heads, so the viewer must hold a real key to sign it — seed one.
  await seedActiveIdentity(page, TEST_IDENTITIES.tyler);
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:release-analyst",
        displayName: "Release Analyst",
        systemPrompt: "Summarise the release.",
      },
    ],
    teams: [
      {
        id: "team-release-010",
        name: "Release Crew",
        description: "Ships the release notes.",
        personaIds: ["custom:release-analyst"],
      },
    ],
  });
  await gotoAgentsView(page);

  await openTeamCatalog(page);
  await expect(page.getByTestId("community-catalog-empty-state")).toBeVisible();
  await page.keyboard.press("Escape");

  await openTeamShareDialog(page, "Release Crew");
  const catalogAccess = page.getByTestId("team-share-catalog-access");
  await expect(catalogAccess).not.toBeChecked();
  await setTeamCatalogAccess(page, true);
  await expect(catalogAccess).toBeChecked();
  await expect(
    page.getByText("Published Release Crew to the community catalog."),
  ).toBeVisible();
  await page
    .getByTestId("team-share-dialog")
    .getByRole("button", { name: "Close" })
    .click();

  await openTeamCatalog(page);
  const ownEntry = page.locator('[data-testid^="community-catalog-team-"]');
  await expect(ownEntry).toHaveCount(1);
  await ownEntry.click();
  await expect(page.getByTestId("community-catalog-detail-pane")).toContainText(
    "Added by You",
  );
  // The publisher already has the team, so the catalog must not offer a copy.
  await expect(page.getByTestId("community-catalog-add-team")).toBeDisabled();
  await page.keyboard.press("Escape");

  await openTeamShareDialog(page, "Release Crew");
  await setTeamCatalogAccess(page, false);
  await expect(
    page.getByText(
      "Release Crew is no longer discoverable in the community catalog.",
    ),
  ).toBeVisible();
  await page
    .getByTestId("team-share-dialog")
    .getByRole("button", { name: "Close" })
    .click();

  await openTeamCatalog(page);
  await expect(page.getByTestId("community-catalog-empty-state")).toBeVisible();
});

test("a queued team share is not presented as relay-published", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:queued-analyst",
        displayName: "Queued Analyst",
        systemPrompt: "Wait for relay acceptance.",
      },
    ],
    teams: [
      {
        id: "team-queued-011",
        name: "Queued Crew",
        description: null,
        personaIds: ["custom:queued-analyst"],
      },
    ],
    teamSharePublicationStatuses: ["queued"],
  });
  await gotoAgentsView(page);

  await openTeamShareDialog(page, "Queued Crew");
  await setTeamCatalogAccess(page, true);
  await expect(
    page.getByText(
      "Sharing Queued Crew is queued. It will appear after the relay accepts the update.",
    ),
  ).toBeVisible();
  await page
    .getByTestId("team-share-dialog")
    .getByRole("button", { name: "Close" })
    .click();

  await openTeamCatalog(page);
  await expect(page.getByTestId("community-catalog-empty-state")).toBeVisible();
});

test("expanding a member row reveals its metadata and instruction", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice's Review Crew",
        members: [
          {
            memberKey: "reviewer",
            displayName: "Alice's Reviewer",
            systemPrompt: "Review changes for the whole community.",
            model: "claude-sonnet-4-5",
            runtime: "claude-code",
            provider: "anthropic",
          },
        ],
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  const memberRow = page.getByTestId("community-catalog-member-reviewer");
  await expect(memberRow).toBeVisible();

  // Metadata card and instruction are hidden before expansion.
  await expect(memberRow.getByTestId("agent-definition-metadata")).toBeHidden();

  // Expand the row.
  await page.getByTestId("community-catalog-member-expand-reviewer").click();

  // aria-expanded transitions to true.
  await expect(
    page.getByTestId("community-catalog-member-expand-reviewer"),
  ).toHaveAttribute("aria-expanded", "true");

  // Metadata card is now visible and contains model/runtime/provider.
  const metadata = memberRow.getByTestId("agent-definition-metadata");
  await expect(metadata).toBeVisible();
  await expect(metadata).toContainText("claude-sonnet-4-5");
  await expect(metadata).toContainText("claude-code");
  await expect(metadata).toContainText("anthropic");

  // Instruction text is visible.
  await expect(memberRow).toContainText(
    "Review changes for the whole community.",
  );
});

test("expanding a member with no system prompt shows the no-instructions placeholder", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice's Review Crew",
        members: [
          {
            memberKey: "reviewer",
            displayName: "Alice's Reviewer",
            systemPrompt: "",
          },
        ],
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  await page.getByTestId("community-catalog-member-expand-reviewer").click();
  await expect(
    page.getByTestId("community-catalog-member-reviewer"),
  ).toContainText("No instructions");
});

test("team instructions section is visible when the team has instructions set", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice's Review Crew",
        instructions: "Always check for security issues first.",
        members: ALICE_TEAM_MEMBERS,
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  const detail = page.getByTestId("community-catalog-detail-pane");
  await expect(detail).toContainText("Team instructions");
  await expect(detail).toContainText("Always check for security issues first.");
});

test("team instructions section is absent when instructions are not set", async ({
  page,
}) => {
  const entryKey = `${TEST_IDENTITIES.alice.pubkey}:${ALICE_TEAM_D_TAG}`;
  await installMockBridge(page, {
    teamCatalogEvents: [
      createTeamCatalogEvent({
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        teamDTag: ALICE_TEAM_D_TAG,
        name: "Alice's Review Crew",
        members: ALICE_TEAM_MEMBERS,
      }),
    ],
  });
  await gotoAgentsView(page);
  await openTeamCatalog(page);
  await page.getByTestId(`community-catalog-team-${entryKey}`).click();

  const detail = page.getByTestId("community-catalog-detail-pane");
  await expect(detail).not.toContainText("Team instructions");
});
