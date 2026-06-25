import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/projects-avatar";
const BRAIN_PUBKEY =
  "1d4f144e07e4c289490acf6d51b50e5450820ee0555783972a22a3074fb1d8bf";
const THOMAS_PUBKEY =
  "29ddeb07aec92535a5b38b7ea1d731bc641fd97ffcf59080ab9a2584d3cbe5c6";
const BRAIN_AVATAR =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='1' y2='1'%3E%3Cstop stop-color='%238b5cf6'/%3E%3Cstop offset='1' stop-color='%2306b6d4'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='64' height='64' rx='32' fill='url(%23g)'/%3E%3Ctext x='32' y='39' text-anchor='middle' font-size='24' font-family='Inter,Arial' fill='white' font-weight='700'%3EB%3C/text%3E%3C/svg%3E";

const PROJECT_ID = `${BRAIN_PUBKEY}:git-ticket-trello`;
const PROJECT = {
  id: PROJECT_ID,
  dtag: "git-ticket-trello",
  name: "Git Ticket Trello Board",
  description: "Trello-style workflow for moving git tickets back and forth.",
  cloneUrls: [
    `https://sprout-oss.stage.blox.sqprod.co/git/${BRAIN_PUBKEY}/git-ticket-trello.git`,
  ],
  webUrl: null,
  owner: BRAIN_PUBKEY,
  contributors: [THOMAS_PUBKEY],
  createdAt: 1_782_389_983,
  projectChannelId: "f147ef69-9ec1-48cf-8e0e-524fb3b33cee",
  status: "active",
  defaultBranch: "main",
  repoAddress: `30617:${BRAIN_PUBKEY}:git-ticket-trello`,
};

const SECOND_PROJECT = {
  ...PROJECT,
  id: `${BRAIN_PUBKEY}:agent-review-queue`,
  dtag: "agent-review-queue",
  name: "Agent Review Queue",
  description: "Track branches, patches, and review notes across agent work.",
  cloneUrls: [
    `https://sprout-oss.stage.blox.sqprod.co/git/${BRAIN_PUBKEY}/agent-review-queue.git`,
  ],
  repoAddress: `30617:${BRAIN_PUBKEY}:agent-review-queue`,
  projectChannelId: null,
  createdAt: 1_782_300_000,
};

const THIRD_PROJECT = {
  ...PROJECT,
  id: `${BRAIN_PUBKEY}:workflow-sandbox`,
  dtag: "workflow-sandbox",
  name: "Workflow Sandbox",
  description: "Prototype board automations before promoting them to staging.",
  cloneUrls: [
    `https://sprout-oss.stage.blox.sqprod.co/git/${BRAIN_PUBKEY}/workflow-sandbox.git`,
  ],
  repoAddress: `30617:${BRAIN_PUBKEY}:workflow-sandbox`,
  status: "draft",
  createdAt: 1_782_200_000,
};

async function seedProjects(page: Page) {
  await page.evaluate(
    ({ brainPubkey, project, secondProject, thomasPubkey, thirdProject }) => {
      window.__BUZZ_E2E_QUERY_CLIENT__?.setQueryData?.(
        ["projects"],
        [project, secondProject, thirdProject],
      );
      window.__BUZZ_E2E_QUERY_CLIENT__?.setQueryData?.(
        ["project", project.dtag],
        project,
      );
      window.__BUZZ_E2E_QUERY_CLIENT__?.setQueryData?.(
        ["project", project.id, "issues"],
        [
          {
            id: "a".repeat(64),
            title: "Move git tickets between Trello columns",
            content:
              "Persist movement through NIP-34 status events and keep history auditable.",
            author: thomasPubkey,
            createdAt: 1_782_389_990,
            repoAddress: project.repoAddress,
            labels: ["feature", "projects"],
            recipients: [brainPubkey],
            status: "In Progress",
            statusEventId: null,
            updatedAt: 1_782_390_100,
          },
          {
            id: "b".repeat(64),
            title: "Render agent avatar in project cards",
            content: "Show Brain's avatar directly inside the agent pill.",
            author: brainPubkey,
            createdAt: 1_782_389_995,
            repoAddress: project.repoAddress,
            labels: ["ui"],
            recipients: [thomasPubkey],
            status: "Done",
            statusEventId: null,
            updatedAt: 1_782_390_200,
          },
        ],
      );
      window.__BUZZ_E2E_QUERY_CLIENT__?.setQueryData?.(
        ["project", project.id, "repo-state"],
        {
          branches: [
            {
              name: "main",
              commit: "0123456789abcdef0123456789abcdef01234567",
            },
            {
              name: "feature/trello-board",
              commit: "fedcba9876543210fedcba9876543210fedcba98",
            },
          ],
          tags: [],
          head: "refs/heads/main",
          updatedAt: 1_782_390_300,
        },
      );
      window.__BUZZ_E2E_QUERY_CLIENT__?.setQueryData?.(
        [
          "projects",
          "activity-summaries",
          [
            project.repoAddress,
            secondProject.repoAddress,
            thirdProject.repoAddress,
          ].sort(),
        ],
        {
          [project.repoAddress]: {
            repoAddress: project.repoAddress,
            issueCount: 2,
            activityCount: 5,
            updatedAt: 1_782_390_300,
            participantPubkeys: [brainPubkey, thomasPubkey],
          },
          [secondProject.repoAddress]: {
            repoAddress: secondProject.repoAddress,
            issueCount: 1,
            activityCount: 2,
            updatedAt: 1_782_300_100,
            participantPubkeys: [brainPubkey],
          },
          [thirdProject.repoAddress]: {
            repoAddress: thirdProject.repoAddress,
            issueCount: 0,
            activityCount: 0,
            updatedAt: 0,
            participantPubkeys: [],
          },
        },
      );
    },
    {
      brainPubkey: BRAIN_PUBKEY,
      project: PROJECT,
      secondProject: SECOND_PROJECT,
      thomasPubkey: THOMAS_PUBKEY,
      thirdProject: THIRD_PROJECT,
    },
  );
}

test.describe("project cards", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("show grid/list modes, agent avatar, delete action, and detail view", async ({
    page,
  }) => {
    await installMockBridge(page, {
      searchProfiles: [
        {
          pubkey: BRAIN_PUBKEY,
          displayName: "Brain",
          avatarUrl: BRAIN_AVATAR,
          isAgent: true,
          ownerPubkey: THOMAS_PUBKEY,
        },
        {
          pubkey: THOMAS_PUBKEY,
          displayName: "Thomas P",
          avatarUrl: null,
        },
      ],
    });

    await page.goto("/");
    await page.waitForFunction(() => Boolean(window.__BUZZ_E2E_QUERY_CLIENT__));
    await page.getByTestId("open-projects-view").click();
    await seedProjects(page);

    const card = page.getByTestId("project-card-git-ticket-trello");
    await expect(card).toBeVisible();
    await expect(card.getByText("Agent: Brain")).toBeVisible();
    await expect(
      card.getByTestId("project-work-owner-avatar-image"),
    ).toBeVisible();

    await card.hover();
    await expect(
      page.getByLabel("Delete Git Ticket Trello Board"),
    ).toBeVisible();

    await waitForAnimations(page);
    await card.screenshot({ path: `${SHOTS}/01-project-grid-card.png` });

    await page.getByRole("button", { name: "List" }).click();
    const row = page.getByTestId("project-row-git-ticket-trello");
    await expect(row).toBeVisible();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/02-project-list-row.png` });

    await row.click();
    await expect(page.getByRole("tab", { name: "Files" })).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: "desktop/src/features/projects/ui/ProjectDetailScreen.tsx",
      }),
    ).toBeVisible();
    await expect(page.getByText("return <WorkspaceTabs")).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/03-project-detail-files-tab.png`,
      clip: { x: 240, y: 64, width: 880, height: 620 },
    });

    await page.getByRole("tab", { name: "Issues" }).click();
    await expect(
      page.getByText("Move git tickets between Trello columns"),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/04-project-detail-issues-tab.png`,
      clip: { x: 240, y: 64, width: 880, height: 620 },
    });
  });
});
