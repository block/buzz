import assert from "node:assert/strict";
import test from "node:test";

import {
  filterClickUpTasks,
  groupClickUpTasks,
  taskUrgencyGroup,
} from "./tasks.ts";

const NOW = new Date(2026, 6, 31, 10, 0, 0);

function task(id, name, dueDateMs, overrides = {}) {
  return {
    id,
    name,
    textContent: "",
    description: "",
    status: { status: "open", color: null, kind: "open" },
    priority: null,
    dueDateMs,
    startDateMs: null,
    dateCreatedMs: null,
    dateUpdatedMs: null,
    archived: false,
    parentId: null,
    url: `https://app.clickup.com/t/${id}`,
    workspaceId: "workspace-1",
    list: { id: "list-1", name: "Inbox" },
    folder: null,
    space: null,
    assignees: [],
    tags: [],
    subtasks: [],
    customFields: [],
    dependencies: [],
    ...overrides,
  };
}

test("taskUrgencyGroup uses local calendar boundaries", () => {
  assert.equal(
    taskUrgencyGroup(
      task("a", "A", String(new Date(2026, 6, 30, 23).getTime())),
      NOW,
    ),
    "overdue",
  );
  assert.equal(
    taskUrgencyGroup(
      task("b", "B", String(new Date(2026, 6, 31, 18).getTime())),
      NOW,
    ),
    "today",
  );
  assert.equal(
    taskUrgencyGroup(
      task("c", "C", String(new Date(2026, 7, 7, 18).getTime())),
      NOW,
    ),
    "next-seven-days",
  );
  assert.equal(
    taskUrgencyGroup(
      task("d", "D", String(new Date(2026, 7, 8, 0).getTime())),
      NOW,
    ),
    "later",
  );
  assert.equal(taskUrgencyGroup(task("e", "E", null), NOW), "no-due-date");
});

test("groupClickUpTasks keeps urgency order data and sorts by due date", () => {
  const earlier = task(
    "earlier",
    "Earlier",
    String(new Date(2026, 6, 31, 12).getTime()),
  );
  const later = task(
    "later",
    "Later",
    String(new Date(2026, 6, 31, 20).getTime()),
  );
  const groups = groupClickUpTasks([later, earlier], NOW);
  assert.deepEqual(
    groups.today.map((item) => item.id),
    ["earlier", "later"],
  );
});

test("filterClickUpTasks combines local name, status, priority, location, and due filters", () => {
  const urgent = task(
    "urgent",
    "Prepare board pack",
    String(new Date(2026, 6, 31, 15).getTime()),
    {
      status: { status: "in progress", color: null, kind: "custom" },
      priority: { priority: "high", color: null },
      list: { id: "ops", name: "Operations" },
    },
  );
  const other = task("other", "Write notes", null);
  const filtered = filterClickUpTasks(
    [urgent, other],
    {
      search: "board",
      status: "in progress",
      priority: "high",
      location: "ops",
      dueWindow: "today",
    },
    NOW,
  );
  assert.deepEqual(
    filtered.map((item) => item.id),
    ["urgent"],
  );
});
