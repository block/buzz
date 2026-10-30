import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test } from "@playwright/test";
import { finalizeEvent } from "nostr-tools/pure";

import type { RelayEvent } from "@/shared/api/types";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

function ownerPrivateKeyFor(pubkey: string): Uint8Array {
  const privateKey = Object.values(TEST_IDENTITIES).find(
    (identity) => identity.pubkey === pubkey,
  )?.privateKey;
  if (!privateKey) {
    throw new Error(`No test private key for ${pubkey}`);
  }
  return hexToBytes(privateKey);
}

const SHOTS = "test-results/team-catalog";

type CatalogMember = {
  memberKey: string;
  displayName: string;
  systemPrompt: string;
  model?: string | null;
  runtime?: string | null;
  provider?: string | null;
};

/** A kind:30178 head, shaped exactly as `team_catalog_content` projects it. */
function createTeamCatalogEvent(input: {
  ownerPubkey: string;
  teamDTag: string;
  name: string;
  description?: string | null;
  instructions?: string | null;
  members: CatalogMember[];
}): RelayEvent {
  return finalizeEvent(
    {
      created_at: 1_721_750_400,
      kind: 30178,
      tags: [
        ["d", input.teamDTag],
        ["shared", "true"],
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
    ownerPrivateKeyFor(input.ownerPubkey),
  );
}

/** A kind:30175 head, shaped exactly as `persona_catalog_content` projects it. */
function createPersonaCatalogEvent(input: {
  ownerPubkey: string;
  sourcePersonaId: string;
  displayName: string;
  systemPrompt: string;
}): RelayEvent {
  return finalizeEvent(
    {
      created_at: 1_721_750_400,
      kind: 30175,
      tags: [
        ["d", input.sourcePersonaId],
        ["shared", "true"],
      ],
      content: JSON.stringify({
        display_name: input.displayName,
        system_prompt: input.systemPrompt,
        avatar_url: null,
        runtime: null,
        model: null,
        provider: null,
        name_pool: [],
      }),
    },
    ownerPrivateKeyFor(input.ownerPubkey),
  );
}

const PERSONA_CATALOG_EVENTS: RelayEvent[] = [
  createPersonaCatalogEvent({
    ownerPubkey: TEST_IDENTITIES.bob.pubkey,
    sourcePersonaId: "code-reviewer",
    displayName: "Code Reviewer",
    systemPrompt: "Review pull requests for correctness and edge cases.",
  }),
];

const CATALOG_EVENTS: RelayEvent[] = [
  createTeamCatalogEvent({
    ownerPubkey: TEST_IDENTITIES.alice.pubkey,
    teamDTag: "release-review",
    name: "Release Review",
    description:
      "Reads the diff, drafts the release note, and files the follow-ups.",
    instructions:
      "Coordinate as a unit. The reviewer and scribe share findings before the triager acts.",
    members: [
      {
        memberKey: "reviewer",
        displayName: "Reviewer",
        systemPrompt: "Review the diff for correctness and edge cases.",
        model: "claude-sonnet-4-5",
        runtime: "claude-code",
        provider: "anthropic",
      },
      {
        memberKey: "scribe",
        displayName: "Scribe",
        systemPrompt: "Write the release note from the merged changes.",
      },
      {
        memberKey: "triager",
        displayName: "Triager",
        systemPrompt: "File follow-ups for anything the review deferred.",
        model: "gpt-5-codex",
      },
    ],
  }),
  createTeamCatalogEvent({
    ownerPubkey: TEST_IDENTITIES.bob.pubkey,
    teamDTag: "incident-desk",
    name: "Incident Desk",
    description: "Two agents that hold the timeline during an incident.",
    members: [
      {
        memberKey: "commander",
        displayName: "Commander",
        systemPrompt: "Own the incident timeline and the comms cadence.",
      },
      {
        memberKey: "investigator",
        displayName: "Investigator",
        systemPrompt: "Chase the root cause and report findings.",
      },
    ],
  }),
];

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
  await waitForAnimations(page);
}

test.describe("team catalog screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("01 — catalog browse, add, and added states", async ({ page }) => {
    test.setTimeout(60_000);
    await installMockBridge(page, { teamCatalogEvents: CATALOG_EVENTS });
    await gotoAgentsView(page);
    await openTeamCatalog(page);

    // 1. Browsing another member's publication: list, provenance, per-member
    //    model. Release Review is not the default selection, so click it.
    const releaseReview = `community-catalog-team-${TEST_IDENTITIES.alice.pubkey}:release-review`;
    await page.getByTestId(releaseReview).click();
    await waitForAnimations(page);
    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-browse.png`,
    });

    // 2. Expand the Reviewer member row to show metadata + instruction.
    await page.getByTestId("community-catalog-member-expand-reviewer").click();
    await waitForAnimations(page);
    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-member-expanded.png`,
    });

    // 3. Team instructions section is visible (Release Review has instructions).
    //    Collapse the expanded member row first so the team-instructions state
    //    is visually distinct from the expanded-member screenshot above.
    await page.getByTestId("community-catalog-member-expand-reviewer").click();
    await waitForAnimations(page);
    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-team-instructions.png`,
    });

    // 4. Adding closes the dialog and names the team in the notice.
    await page.getByTestId("community-catalog-add-team").click();
    await expect(
      page.getByText("Added Release Review to your teams."),
    ).toBeVisible();

    // 5. Reopened: the action reads "Added to my teams" and is inert, so a
    //    second copy of the same publication is not offered.
    await openTeamCatalog(page);
    await page.getByTestId(releaseReview).click();
    await expect(page.getByTestId("community-catalog-add-team")).toHaveText(
      "Added to my teams",
    );
    await waitForAnimations(page);
    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-added.png`,
    });
  });

  test("02 — empty catalog", async ({ page }) => {
    await installMockBridge(page, { teamCatalogEvents: [] });
    await gotoAgentsView(page);
    await openTeamCatalog(page);

    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-empty.png`,
    });
  });

  test("03 — share dialog before and after publishing", async ({ page }) => {
    test.setTimeout(60_000);
    await installMockBridge(page, {
      personas: [
        {
          id: "custom:release-analyst",
          displayName: "Release Analyst",
          systemPrompt: "Summarise the release.",
        },
        {
          id: "custom:release-scribe",
          displayName: "Release Scribe",
          systemPrompt: "Write the release note.",
        },
      ],
      teams: [
        {
          id: "team-release-010",
          name: "Release Crew",
          description: "Ships the release notes.",
          personaIds: ["custom:release-analyst", "custom:release-scribe"],
        },
      ],
    });
    await gotoAgentsView(page);

    await page.getByLabel("Release Crew team actions").click();
    await page.getByRole("menuitem", { name: "Share" }).click();
    await expect(page.getByTestId("team-share-dialog")).toBeVisible();
    await waitForAnimations(page);

    // 4. Catalog access sits alongside the existing snapshot-share controls,
    //    defaulting to unchecked (not shared).
    await page.getByTestId("team-share-dialog").screenshot({
      path: `${SHOTS}/share-not-shared.png`,
    });

    // 5. Published: toggle the Switch to share, the toast names the effect.
    await page.getByTestId("team-share-catalog-access").click();
    await expect(page.getByTestId("team-share-catalog-access")).toBeChecked();
    await expect(
      page.getByText("Published Release Crew to the community catalog."),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/share-published.png` });
  });

  test("04 — both sections populated (agents + teams)", async ({ page }) => {
    await installMockBridge(page, {
      personaCatalogEvents: PERSONA_CATALOG_EVENTS,
      teamCatalogEvents: CATALOG_EVENTS,
    });
    await gotoAgentsView(page);
    await openTeamCatalog(page);

    // The Agents section header is visible alongside Teams in the sidebar.
    await expect(
      page.locator('[data-testid^="community-catalog-agent-"]'),
    ).toHaveCount(1);
    await expect(
      page.locator('[data-testid^="community-catalog-team-"]'),
    ).toHaveCount(2);
    await waitForAnimations(page);
    await page.getByTestId("community-catalog-dialog").screenshot({
      path: `${SHOTS}/catalog-both-sections.png`,
    });
  });
});
