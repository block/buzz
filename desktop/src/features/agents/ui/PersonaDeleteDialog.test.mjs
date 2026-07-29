import assert from "node:assert/strict";
import test from "node:test";

import {
  AlertDialogDescription,
  AlertDialogHeader,
} from "@/shared/ui/alert-dialog.tsx";

import {
  PersonaDeleteDialog,
  personaDeleteDescription,
  personaDeleteRemoteWarning,
} from "./PersonaDeleteDialog.tsx";

// Regression guard for the persona-cascade consent copy: deleting a persona
// with instances also archives each instance's identity on the relay
// (NIP-IA 9035), a durable externally visible side effect. The confirmation
// dialog must disclose it before the destructive confirm, exactly like the
// direct agent-delete dialog does.

const persona = { displayName: "Scout" };

test("cascade delete discloses relay archival (plural)", () => {
  const copy = personaDeleteDescription(persona, 3);
  assert.match(copy, /deletes 3 agent instances/);
  assert.match(copy, /archives their identities on the relay/);
});

test("cascade delete discloses relay archival (singular)", () => {
  const copy = personaDeleteDescription(persona, 1);
  assert.match(copy, /deletes 1 agent instance /);
  assert.match(copy, /archives its identity on the relay/);
});

test("no instances → no archival claim (nothing is archived)", () => {
  const copy = personaDeleteDescription(persona, 0);
  assert.equal(copy, "Delete Scout.");
  assert.doesNotMatch(copy, /archiv/i);
});

test("null persona keeps the generic fallback", () => {
  assert.equal(personaDeleteDescription(null, 2), "Delete this agent.");
});

// The cascade's remote-orphan disclosure. Deleting a persona whose instances
// are provider-deployed does not stop those deployments — the provider
// protocol has no undeploy — so the dialog must say that plainly and name
// each unit, since naming it is the only way the owner can act on it.

const scout = { name: "Remote Scout", unitId: "buzz-agent-scout.service" };
const relay = { name: "Relay Watcher", unitId: "buzz-agent-relay.service" };

test("remote warning names the unit and refuses to imply a teardown", () => {
  const copy = personaDeleteRemoteWarning([scout]);
  assert.match(copy, /1 of these instances is deployed remotely/);
  assert.match(copy, /Remote Scout \(buzz-agent-scout\.service\)/);
  assert.match(copy, /does not stop them/);
  assert.match(copy, /keep running/);
});

test("remote warning lists every orphaned unit", () => {
  const copy = personaDeleteRemoteWarning([scout, relay]);
  assert.match(copy, /2 of these instances are deployed remotely/);
  assert.match(copy, /buzz-agent-scout\.service/);
  assert.match(copy, /buzz-agent-relay\.service/);
});

test("no remote instances → no warning at all", () => {
  // A hedged "may be running" on a local-only cascade would train the owner
  // to dismiss the warning that matters.
  assert.equal(personaDeleteRemoteWarning([]), null);
});

// DOM-shape guards. The warning cannot be a second AlertDialogDescription:
// Radix derives one aria-describedby id per Content, so a second Description
// is announced to nobody — the one sentence that reports an irreversible
// remote side effect would be silent for screen-reader users. It also has to
// stay outside AlertDialogHeader, which is where the sibling
// AgentDeleteConfirmDialog puts its equivalent copy. Rendering goes through
// a Radix portal, so markup assertions see an empty string here; walk the
// element tree the component returns instead.

function collect(node, found) {
  if (Array.isArray(node)) {
    for (const child of node) {
      collect(child, found);
    }
    return found;
  }
  if (!node || typeof node !== "object" || !("type" in node)) {
    return found;
  }
  found.push(node);
  collect(node.props?.children, found);
  return found;
}

function renderTree(remoteInstances) {
  return collect(
    PersonaDeleteDialog({
      instanceCount: 2,
      onConfirm: () => {},
      onOpenChange: () => {},
      open: true,
      persona: { displayName: "Scout" },
      remoteInstances,
    }),
    [],
  );
}

function findWarning(nodes) {
  return nodes.find(
    (node) => node.props?.["data-testid"] === "persona-delete-remote-warning",
  );
}

test("remote warning renders as a plain <p>, not a second Description", () => {
  const nodes = renderTree([scout]);

  const warning = findWarning(nodes);
  assert.ok(warning, "remote-orphan warning must be rendered");
  assert.equal(
    warning.type,
    "p",
    "warning must be a plain <p>; a second AlertDialogDescription is never announced",
  );
  assert.match(warning.props.className, /text-destructive/);
  assert.equal(warning.props.children, personaDeleteRemoteWarning([scout]));

  const descriptions = nodes.filter(
    (node) => node.type === AlertDialogDescription,
  );
  assert.equal(
    descriptions.length,
    1,
    "exactly one AlertDialogDescription may exist per AlertDialogContent",
  );
});

test("remote warning sits outside AlertDialogHeader", () => {
  const header = renderTree([scout]).find(
    (node) => node.type === AlertDialogHeader,
  );
  assert.ok(header, "dialog must still render a header");
  assert.equal(
    findWarning(collect(header.props.children, [])),
    undefined,
    "warning must be a sibling of the header, matching AgentDeleteConfirmDialog",
  );
});

test("local-only cascade renders no warning element", () => {
  assert.equal(findWarning(renderTree([])), undefined);
});
