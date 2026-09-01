import assert from "node:assert/strict";
import test from "node:test";

import {
  canOpenProjectChannelDialog,
  projectsAvailableForChannelCreation,
  shouldLoadProjectHomeRoster,
} from "./ProjectsCategoryCreateDialogs.tsx";

const HOME = "11111111-1111-4111-8111-111111111111";

function project(id, overrides = {}) {
  return {
    id,
    legacy: false,
    owner: "owner",
    projectChannelId: null,
    ...overrides,
  };
}

test("direct and controlled-owner Projects remain channel targets without a home", () => {
  const direct = project("direct");
  const controlled = project("controlled");
  assert.equal(
    canOpenProjectChannelDialog([direct, controlled], [direct, controlled]),
    true,
  );
  assert.deepEqual(
    projectsAvailableForChannelCreation(
      [direct, controlled, project("uncontrolled")],
      new Set([direct.id, controlled.id]),
    ).map((candidate) => candidate.id),
    [direct.id, controlled.id],
  );
});

test("home Projects remain selectable for a lazy role check", () => {
  const homeProject = project("home", { projectChannelId: HOME });
  assert.deepEqual(
    projectsAvailableForChannelCreation([homeProject], new Set()),
    [homeProject],
  );
  assert.equal(
    shouldLoadProjectHomeRoster(homeProject, true, false, "viewer"),
    true,
  );
  assert.equal(
    shouldLoadProjectHomeRoster(homeProject, true, true, "viewer"),
    false,
  );
});

test("legacy and unowned no-home Projects are not channel targets", () => {
  assert.equal(
    canOpenProjectChannelDialog(
      [project("legacy", { legacy: true }), project("unowned")],
      [],
    ),
    false,
  );
});
