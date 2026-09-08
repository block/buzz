import assert from "node:assert/strict";
import test from "node:test";

import {
  compactWorkflowTriggerDescription,
  splitWorkflowAuthorDescription,
  workflowTriggerDescription,
} from "./workflowTriggerDescription.ts";

test("splits generated author attribution instead of matching quoted user copy", () => {
  assert.deepEqual(
    splitWorkflowAuthorDescription(
      "Reaction added by Carl to “thanks Carl”",
      "Carl",
    ),
    { prefix: "Reaction added by", suffix: "to “thanks Carl”" },
  );
  assert.deepEqual(
    splitWorkflowAuthorDescription("Message by Carl contains “Carl”", "Carl"),
    { prefix: "Message by", suffix: "contains “Carl”" },
  );
  assert.deepEqual(
    splitWorkflowAuthorDescription(
      "Message by anyone except Carl contains “Carl”",
      "Carl",
    ),
    { prefix: "Message by anyone except", suffix: "contains “Carl”" },
  );
  assert.equal(
    splitWorkflowAuthorDescription("Message contains “Carl”", "Carl"),
    null,
  );
});

test("describes selected trigger conditions on the workflow canvas", () => {
  assert.equal(
    workflowTriggerDescription(
      {
        on: "message_posted",
        filter: `trigger_author == "${"a".repeat(64)}"`,
      },
      { authorLabel: "Carl" },
    ),
    "Message posted by Carl",
  );

  assert.equal(
    workflowTriggerDescription(
      {
        on: "message_posted",
        filter: `trigger_author == "${"a".repeat(64)}"`,
      },
      { authorLoading: true },
    ),
    "Message posted by loading author",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: 'str_contains(trigger_text, "deploy")',
    }),
    "Message contains “deploy”",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: '!str_contains(trigger_text, "deploy")',
    }),
    "Message doesn’t contain “deploy”",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: 'str_starts_with(trigger_text, "deploy")',
    }),
    "Message starts with “deploy”",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: 'str_ends_with(trigger_text, "done")',
    }),
    "Message ends with “done”",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: 'trigger_text == "deploy"',
    }),
    "Message “deploy” is posted",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: 'trigger_text != "deploy"',
    }),
    "Message with text other than “deploy” posted",
  );

  assert.equal(
    workflowTriggerDescription(
      {
        on: "message_posted",
        filter: `trigger_text == "FUCK" && trigger_author == "${"a".repeat(64)}"`,
      },
      { authorLabel: "Carl" },
    ),
    "Message “FUCK” is posted by Carl",
  );

  assert.equal(
    workflowTriggerDescription(
      {
        on: "message_posted",
        filter: `str_contains(trigger_text, "deploy") && trigger_author == "${"a".repeat(64)}"`,
      },
      { authorLabel: "Carl" },
    ),
    "Message by Carl contains “deploy”",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: "str_len(trigger_text) == 0",
    }),
    "Message without text posted",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: "str_len(trigger_text) > 0",
    }),
    "Message with text posted",
  );

  assert.equal(
    workflowTriggerDescription({
      on: "reaction_added",
      filter: 'trigger_emoji == "🔥"',
    }),
    "🔥 reaction added",
  );

  assert.equal(
    workflowTriggerDescription(
      {
        on: "reaction_added",
        filter: `trigger_message_id == "${"b".repeat(64)}"`,
      },
      { messageLoading: true },
    ),
    "Reaction added to loading message",
  );

  assert.equal(
    workflowTriggerDescription(
      {
        on: "reaction_added",
        filter: `trigger_emoji == "👾" && trigger_author == "${"a".repeat(64)}" && trigger_message_id == "${"b".repeat(64)}"`,
      },
      { authorLabel: "Carl", messageLabel: "hey yourself" },
    ),
    "👾 reaction added by Carl to “hey yourself”",
  );
});

test("compacts only an included emoji already rendered as the node icon", () => {
  assert.equal(
    compactWorkflowTriggerDescription("😍 reaction added", "😍"),
    "Reaction added",
  );
  assert.equal(
    compactWorkflowTriggerDescription("😍 reaction added by Carl", "😍"),
    "Reaction added by Carl",
  );
  assert.equal(
    compactWorkflowTriggerDescription(
      "😍 reaction added by Carl to “release 😍 notes”",
      "😍",
    ),
    "Reaction added by Carl to “release 😍 notes”",
  );
  assert.equal(
    compactWorkflowTriggerDescription(
      "Any reaction except 😍 added",
      undefined,
    ),
    "Any reaction except 😍 added",
  );
  assert.equal(
    compactWorkflowTriggerDescription(
      "Reaction added to “😍 reaction added”",
      "😍",
    ),
    "Reaction added to “😍 reaction added”",
  );
});

test("keeps the base label for unfiltered and custom triggers", () => {
  assert.equal(
    workflowTriggerDescription({ on: "message_posted" }),
    "Message posted",
  );
  assert.equal(
    workflowTriggerDescription({
      on: "message_posted",
      filter: "custom_variable == 1",
    }),
    "Message posted",
  );
});
