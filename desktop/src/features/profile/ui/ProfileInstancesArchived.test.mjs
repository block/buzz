/**
 * Archived-instances subsection regression: the profile panel's Instances list
 * splits live and relay-archived siblings, rendering archived rows under a
 * clearly labeled "Archived" header so unarchive stays UI-reachable for
 * channel-less agents. The section appears whenever live OR archived instances
 * exist, self-omits when neither does, shows only the Archived subsection for
 * an all-archived persona, and archived rows keep the deliberate explicit-
 * pubkey click that feeds selector matrix rule 3.
 *
 * Mounts the shipping ProfileInstancesSection (owned by the Runtime tab) and
 * drives the real expand toggle rather than reimplementing it.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let ProfileInstancesSection;

const LIVE_PK = "b".repeat(64);
const SECOND_LIVE_PK = "c".repeat(64);
const ARCHIVED_PK = "a".repeat(64);

function agent(overrides = {}) {
  return {
    pubkey: LIVE_PK,
    name: "Instance",
    personaId: "persona-1",
    status: "stopped",
    ...overrides,
  };
}

function baseProps(overrides = {}) {
  return {
    currentPubkey: null,
    instances: [],
    archivedInstances: [],
    onOpenInstance: () => {},
    ...overrides,
  };
}

function renderSection(props) {
  return render(createElement(ProfileInstancesSection, baseProps(props)));
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });

  ({ cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ ProfileInstancesSection } = await import("./ProfileInstancesSection.tsx"));
});

afterEach(() => cleanup?.());
after(() => dom.window.close());

test("test_archived_and_live_instances_render_archived_subsection", () => {
  renderSection({
    instances: [agent({ pubkey: LIVE_PK, name: "Live" })],
    archivedInstances: [agent({ pubkey: ARCHIVED_PK, name: "Archived one" })],
  });
  fireEvent.click(screen.getByTestId("user-profile-instances"));

  assert.equal(
    screen.getByTestId("user-profile-instances").textContent,
    "2 instances",
  );
  assert.ok(screen.getByTestId("user-profile-instances-archived-header"));
  assert.ok(screen.getByTestId(`user-profile-instance-${LIVE_PK}`));
  assert.ok(screen.getByTestId(`user-profile-instance-${ARCHIVED_PK}`));
});

test("test_no_archived_instances_omits_archived_subsection", () => {
  renderSection({
    instances: [
      agent({ pubkey: LIVE_PK, name: "Live" }),
      agent({ pubkey: SECOND_LIVE_PK, name: "Live two" }),
    ],
    archivedInstances: [],
  });
  fireEvent.click(screen.getByTestId("user-profile-instances"));

  assert.equal(screen.queryByTestId("user-profile-instances-archived"), null);
  assert.ok(screen.getByTestId(`user-profile-instance-${LIVE_PK}`));
});

test("test_all_archived_persona_shows_only_archived_subsection", () => {
  renderSection({
    instances: [],
    archivedInstances: [agent({ pubkey: ARCHIVED_PK, name: "Archived only" })],
  });
  // Section renders even with an empty live list.
  fireEvent.click(screen.getByTestId("user-profile-instances"));

  assert.equal(
    screen.getByTestId("user-profile-instances").textContent,
    "1 instance",
  );
  assert.ok(screen.getByTestId("user-profile-instances-archived-header"));
  assert.ok(screen.getByTestId(`user-profile-instance-${ARCHIVED_PK}`));
});

test("test_no_instances_at_all_omits_instances_section", () => {
  renderSection({ instances: [], archivedInstances: [] });
  assert.equal(screen.queryByTestId("user-profile-instances-section"), null);
});

test("test_archived_row_click_opens_that_explicit_pubkey", () => {
  const opened = [];
  renderSection({
    instances: [],
    archivedInstances: [agent({ pubkey: ARCHIVED_PK, name: "Archived only" })],
    onOpenInstance: (pubkey) => opened.push(pubkey),
  });
  fireEvent.click(screen.getByTestId("user-profile-instances"));
  fireEvent.click(screen.getByTestId(`user-profile-instance-${ARCHIVED_PK}`));

  // The archived row keeps the deliberate explicit-pubkey path (unarchive).
  assert.deepEqual(opened, [ARCHIVED_PK]);
});
