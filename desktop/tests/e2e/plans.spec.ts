import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

function deploymentPlanDocument() {
  const rows = [
    [
      "WBS",
      "Task",
      "Owner",
      "Start",
      "Due",
      "Duration",
      "Progress",
      "Dependencies",
    ],
    [
      "1",
      "Define support concept",
      "Operations Officer",
      "2026-08-03",
      "2026-08-04",
      "2",
      "0%",
      "",
    ],
    [
      "2",
      "Confirm logistics support",
      "Logistics Officer",
      "2026-08-05",
      "2026-08-07",
      "3",
      "0%",
      "1",
    ],
    [
      "3",
      "Confirm port services",
      "Logistics Officer",
      "2026-08-05",
      "2026-08-05",
      "1",
      "0%",
      "1",
    ],
    [
      "4",
      "Command readiness review",
      "Commanding Officer",
      "2026-08-10",
      "2026-08-10",
      "1",
      "0%",
      "2,3",
    ],
  ];
  return {
    filename: "NT Planning.xlsx",
    extension: "xlsx",
    sha256: "d".repeat(64),
    sizeBytes: 4096,
    blocks: rows.map((cells, index) => ({
      kind: "table_row",
      location: `Planning table row ${index + 1}`,
      cells,
    })),
    pages: [],
    sheets: [],
    truncated: false,
  };
}

async function addTask(
  page: Page,
  input: {
    title: string;
    wbs: string;
    duration: number;
    dependencies?: string[];
  },
) {
  const screen = page.getByTestId("plan-detail-screen");
  await screen.getByRole("button", { name: "Add task" }).click();
  const dialog = page.getByRole("dialog", { name: "New planning task" });
  await dialog.getByLabel("Task").fill(input.title);
  await dialog.getByLabel("WBS").fill(input.wbs);
  await dialog.getByLabel("Owner").fill("Operations Officer");
  await dialog.getByLabel("Start", { exact: true }).fill("2026-08-03");
  await dialog.getByLabel("Due").fill("2026-08-10");
  await dialog
    .getByLabel("Duration (working days)")
    .fill(String(input.duration));
  for (const dependency of input.dependencies ?? [])
    await dialog.getByLabel(dependency).check();
  await dialog.getByRole("button", { name: "Save task" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(screen.getByText(input.title, { exact: true })).toBeVisible();
}

test("Plans persists a deployment network, critical path, and mission constraint", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-plans-view").click();
  await expect(page).toHaveURL(/#\/plans$/);
  const list = page.getByTestId("plans-screen");
  await list.getByRole("button", { name: "New Plan" }).click();
  const create = page.getByRole("dialog", { name: "New operational plan" });
  await create.getByLabel("Plan title").fill("Regional Logistics Deployment");
  await create
    .getByLabel("Purpose")
    .fill("Sustain the task group through the deployment.");
  await create.getByLabel("Mission-ready date").fill("2026-08-10");
  await create.getByRole("button", { name: "Create plan" }).click();

  await expect(page).toHaveURL(/#\/plans\/.+/);
  const detail = page.getByTestId("plan-detail-screen");
  await expect(
    detail.getByRole("heading", { name: "Regional Logistics Deployment" }),
  ).toBeVisible();
  await addTask(page, {
    title: "A – Define support concept",
    wbs: "1",
    duration: 2,
  });
  await addTask(page, {
    title: "B – Confirm logistics support",
    wbs: "2",
    duration: 3,
    dependencies: ["1 A – Define support concept"],
  });
  await addTask(page, {
    title: "C – Confirm port services",
    wbs: "3",
    duration: 1,
    dependencies: ["1 A – Define support concept"],
  });
  await addTask(page, {
    title: "D – Command readiness review",
    wbs: "4",
    duration: 1,
    dependencies: [
      "2 B – Confirm logistics support",
      "3 C – Confirm port services",
    ],
  });

  const gantt = page.getByTestId("gantt-chart");
  await expect(gantt.getByText("6 working days")).toBeVisible();
  await expect(gantt.getByText("On calculated critical path")).toHaveCount(3);
  await expect(gantt.getByText("2 working days float")).toBeVisible();

  await detail
    .getByText("B – Confirm logistics support", { exact: true })
    .click();
  const edit = page.getByRole("dialog", { name: "Edit planning task" });
  await edit.getByLabel("Duration (working days)").fill("4");
  await edit.getByRole("button", { name: "Save task" }).click();
  await expect(gantt.getByText("7 working days")).toBeVisible();
  await expect(gantt.getByText("Mission ready at risk")).toBeVisible();

  await detail.getByRole("button", { name: "Add constraint" }).click();
  const constraint = page.getByRole("dialog", {
    name: "New mission constraint",
  });
  await constraint
    .getByLabel("Constraint and operational effect")
    .fill("Port seaboat davit unserviceable");
  await constraint.getByLabel("Owner").fill("Marine Engineer Officer");
  await constraint.getByLabel("Severity").selectOption("critical");
  await constraint
    .getByLabel("Linked task")
    .selectOption({ label: "2 B – Confirm logistics support" });
  await constraint
    .getByLabel("Mission requirement")
    .fill("Conduct seaboat operations");
  await constraint.getByRole("button", { name: "Save constraint" }).click();
  const panel = page.getByTestId("mission-constraints");
  await expect(
    panel.getByText("Port seaboat davit unserviceable"),
  ).toBeVisible();
  await expect(panel.getByText(/On calculated critical path/)).toBeVisible();
  await detail
    .getByText("B – Confirm logistics support", { exact: true })
    .click();
  const completeTask = page.getByRole("dialog", {
    name: "Edit planning task",
  });
  await completeTask.getByLabel("Status").selectOption("complete");
  await completeTask.getByLabel("Completion").fill("100");
  await completeTask.getByRole("button", { name: "Save task" }).click();
  await expect(panel.getByText("open", { exact: true })).toBeVisible();

  await panel
    .getByRole("button", { name: /Port seaboat davit unserviceable/ })
    .click();
  const disposition = page.getByRole("dialog", {
    name: "Update mission constraint",
  });
  await disposition
    .getByLabel("Disposition", { exact: true })
    .selectOption("resolved");
  await disposition.getByRole("button", { name: "Save constraint" }).click();
  await expect(panel.getByText("resolved", { exact: true })).toBeVisible();
});

test("Plans reviews and imports a deployment WBS from a planning document", async ({
  page,
}) => {
  await installMockBridge(page, {
    battleRhythmDocuments: [deploymentPlanDocument()],
  });
  await page.goto("/");
  await page.getByTestId("open-plans-view").click();
  await page.getByRole("button", { name: "New Plan" }).click();
  const create = page.getByRole("dialog", { name: "New operational plan" });
  await create.getByLabel("Plan title").fill("Imported deployment plan");
  await create
    .getByLabel("Purpose")
    .fill("Prepare the logistics mission through a reviewed WBS.");
  await create.getByLabel("Mission-ready date").fill("2026-08-10");
  await create.getByRole("button", { name: "Create plan" }).click();

  const detail = page.getByTestId("plan-detail-screen");
  await detail.getByRole("button", { name: "Import Plan" }).click();
  const review = page.getByRole("dialog", {
    name: "Review deployment plan import",
  });
  await review
    .getByRole("button", { name: "Choose Word, Excel, or PDF" })
    .click();
  await expect(review.getByText("NT Planning.xlsx")).toBeVisible();
  await expect(review.getByText("4 tasks proposed")).toBeVisible();
  await expect(review.getByText("Confirm logistics support")).toBeVisible();
  await review.getByRole("button", { name: "Import reviewed tasks" }).click();

  await expect(review).toHaveCount(0);
  await expect(
    detail.getByRole("cell", { name: "Define support concept" }),
  ).toBeVisible();
  await expect(
    detail.getByRole("cell", { name: "Command readiness review" }),
  ).toBeVisible();
  const gantt = detail.getByTestId("gantt-chart");
  await expect(gantt.getByText("6 working days")).toBeVisible();
  await expect(gantt.getByText("2 working days float")).toBeVisible();
});
