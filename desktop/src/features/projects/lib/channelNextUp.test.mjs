import assert from "node:assert/strict";
import test from "node:test";
import {
  channelNextUpItems,
  channelNextUpProjects,
  filterChannelNextUpItems,
} from "./channelNextUp.ts";

const PERSON = "a".repeat(64);
const OTHER = "b".repeat(64);
const repository = {
  id: "repo",
  repoAddress: `30617:${PERSON}:repo`,
  channelId: "repo-channel",
};
function project(id, overrides = {}) {
  return {
    id,
    projectChannelId: "home",
    relatedChannelIds: ["related"],
    repositories: [repository],
    ...overrides,
  };
}
function issue(id, overrides = {}) {
  return {
    id,
    channelId: "related",
    repoAddress: repository.repoAddress,
    status: "Backlog",
    assignees: [],
    ...overrides,
  };
}

test("recognizes home, related and repository bindings without admitting unrelated projects", () => {
  const projects = [project("one"), project("empty", { repositories: [] })];
  for (const channel of ["home", "related", "repo-channel"]) {
    assert.deepEqual(
      channelNextUpProjects(projects, channel).map((p) => p.id),
      ["one"],
    );
  }
  assert.deepEqual(channelNextUpProjects(projects, "unknown"), []);
});

test("deduplicates shared repository tasks and retains the correct navigation context", () => {
  const root = issue("one");
  const items = channelNextUpItems(
    [project("one"), project("two")],
    [root, root],
    "related",
  );
  assert.equal(items.length, 1);
  assert.equal(items[0].project.id, "one");
  assert.equal(items[0].repository.id, "repo");
});

test("a matching origin cannot enroll an unrelated or inaccessible repository", () => {
  const items = channelNextUpItems(
    [project("one")],
    [
      issue("visible"),
      issue("other", { repoAddress: "unknown" }),
      issue("missing", { channelId: null }),
      issue("elsewhere", { channelId: "elsewhere" }),
    ],
    "related",
  );
  assert.deepEqual(
    items.map(({ issue: task }) => task.id),
    ["visible"],
  );
  assert.deepEqual(
    channelNextUpItems(
      [project("one")],
      [issue("fake", { channelId: "unknown" })],
      "unknown",
    ),
    [],
  );
});

test("All retains unassigned tasks; person filters match exact keys and every assignee", () => {
  const items = channelNextUpItems(
    [project("one")],
    [
      issue("unassigned"),
      issue("mine", { assignees: [PERSON] }),
      issue("shared", { assignees: [OTHER, PERSON.toUpperCase()] }),
      issue("theirs", { assignees: [OTHER] }),
    ],
    "related",
  );
  assert.equal(filterChannelNextUpItems(items, null).length, 4);
  assert.deepEqual(
    filterChannelNextUpItems(items, PERSON).map(({ issue: task }) => task.id),
    ["mine", "shared"],
  );
  assert.deepEqual(
    filterChannelNextUpItems(items, OTHER).map(({ issue: task }) => task.id),
    ["shared", "theirs"],
  );
  assert.deepEqual(filterChannelNextUpItems(items, "different"), []);
});

test("complete status excludes Done/Closed, but unknown status never suppresses tasks", () => {
  const items = channelNextUpItems(
    [project("one")],
    [
      issue("backlog"),
      issue("triage", { status: "Triage" }),
      issue("working", { status: "In Progress" }),
      issue("review", { status: "In Review" }),
      issue("done", { status: "Done" }),
      issue("closed", { status: "Closed" }),
    ],
    "related",
  );
  assert.equal(filterChannelNextUpItems(items, null).length, 4);
  assert.equal(filterChannelNextUpItems(items, null, false).length, 6);
});
