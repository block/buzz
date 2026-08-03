import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveVoiceTurnRecipient,
  shouldForwardVoiceTurn,
} from "./voiceTurnRouting.ts";

test("forwards every room turn to Orchestrator", () => {
  assert.equal(
    shouldForwardVoiceTurn("Please review this", "Orchestrator"),
    true,
  );
});

test("keeps an unaddressed human turn away from a specialist", () => {
  assert.equal(shouldForwardVoiceTurn("Please review this", "Builder"), false);
});

test("forwards a turn when the specialist is explicitly named", () => {
  assert.equal(
    shouldForwardVoiceTurn(
      "Builder, please implement the approved plan",
      "Builder",
    ),
    true,
  );
});

test("forwards natural questions and delegated requests", () => {
  assert.equal(
    shouldForwardVoiceTurn("What does Architect think?", "Architect"),
    true,
  );
  assert.equal(
    shouldForwardVoiceTurn("Could Builder implement this?", "Builder"),
    true,
  );
  assert.equal(shouldForwardVoiceTurn("Tell Builder to stop", "Builder"), true);
  assert.equal(
    shouldForwardVoiceTurn("Can you invite Researcher now?", "Researcher"),
    true,
  );
});

test("does not wake specialists for narration or quoted examples", () => {
  assert.equal(
    shouldForwardVoiceTurn(
      "Orchestrator, Builder, and Explorer are all listening",
      "Builder",
    ),
    false,
  );
  assert.equal(
    shouldForwardVoiceTurn(
      "So Builder join and Builder stop should change participation immediately",
      "Builder",
    ),
    false,
  );
  assert.equal(
    shouldForwardVoiceTurn(
      "Builder is the primary implementation agent",
      "Builder",
    ),
    false,
  );
});

test("does not match a specialist name inside another word", () => {
  assert.equal(
    shouldForwardVoiceTurn("Use the form builder", "Builder Pro"),
    false,
  );
});

const room = [
  { agentName: "Orchestrator", threadId: "orchestrator" },
  { agentName: "Builder", threadId: "builder" },
  { agentName: "Architect", threadId: "architect" },
];

test("routes an unaddressed turn only to Orchestrator", () => {
  assert.equal(
    resolveVoiceTurnRecipient("Please review this", room)?.threadId,
    "orchestrator",
  );
});

test("routes an explicitly addressed turn to one specialist", () => {
  assert.equal(
    resolveVoiceTurnRecipient("Architect, what do you think?", room)?.threadId,
    "architect",
  );
});

test("routes a name-led concise request from live speech to the specialist", () => {
  assert.equal(
    resolveVoiceTurnRecipient(
      "Architect in one sentence is programmatic voice room join end to end ready and why",
      room,
    )?.threadId,
    "architect",
  );
});
