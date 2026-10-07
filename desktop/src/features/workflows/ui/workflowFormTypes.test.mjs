import assert from "node:assert/strict";
import test from "node:test";
import { parse as parseYaml } from "yaml";

import {
  formStateToYaml,
  isThreadReplyEligibleTrigger,
  supportsMessageTextCondition,
  withTriggerType,
  yamlToFormState,
  DEFAULT_FORM_STATE,
} from "./workflowFormTypes.ts";

function accepted(yaml) {
  const result = yamlToFormState(yaml);
  assert.equal(result.ok, true, result.ok ? undefined : result.error);
  return result.state;
}

function normalizeBackendDefaults(value) {
  const copy = structuredClone(value);
  if (copy.enabled === undefined) copy.enabled = true;
  for (const step of copy.steps ?? []) {
    if (step.action === "call_webhook" && step.method === undefined) {
      step.method = "POST";
    }
  }
  return copy;
}

function sendMessageState(overrides) {
  return {
    ...DEFAULT_FORM_STATE,
    name: "Auto Reply",
    trigger: { on: "message_posted", filter: "trigger_is_reply == false" },
    steps: [
      {
        id: "step_1",
        action: "send_message",
        text: "pre-written reply",
        ...overrides,
      },
    ],
  };
}

test("message-text conditions are limited to message-bearing triggers", () => {
  assert.equal(supportsMessageTextCondition("message_posted"), true);
  assert.equal(supportsMessageTextCondition("diff_posted"), true);
  assert.equal(supportsMessageTextCondition("reaction_added"), false);
  assert.equal(supportsMessageTextCondition("webhook"), false);
  assert.equal(supportsMessageTextCondition("schedule"), false);
});

const acceptedFixtures = [
  `name: Notify\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify_1\n    action: send_message\n    text: hello\n`,
  `name: React\ndescription: React to a message\nenabled: false\ntrigger:\n  on: reaction_added\n  emoji: eyes\n  filter: trigger_message_id == "abc123"\nsteps:\n  - id: react\n    name: Add reaction\n    timeout_secs: 30\n    action: add_reaction\n    emoji: white_check_mark\n`,
  `name: Webhook\ntrigger:\n  on: webhook\nsteps:\n  - id: call\n    action: call_webhook\n    url: https://example.com/hook\n    method: PATCH\n    headers:\n      Authorization: secret\n      X-Trace: trace\n    body: '{"ok":true}'\n`,
  `name: Legacy actions\ntrigger:\n  on: diff_posted\n  filter: str_contains(trigger_text, "deploy")\nsteps:\n  - id: dm\n    action: send_dm\n    to: abc123\n    text: hello\n  - id: approval\n    action: request_approval\n    from: manager\n    message: Approve?\n    timeout: 24h\n  - id: topic\n    action: set_channel_topic\n    topic: Deployed\n  - id: wait\n    action: delay\n    duration: 5m\n`,
  `name: Scheduled preset\ntrigger:\n  on: schedule\n  interval: 15m\nsteps:\n  - id: notify\n    action: send_message\n    text: hello\n`,
  `name: Scheduled custom\ntrigger:\n  on: schedule\n  cron: 0 */2 * * 1,3,5\nsteps:\n  - id: notify\n    action: send_message\n    text: hello\n`,
  `name: Scheduled legacy interval\ntrigger:\n  on: schedule\n  interval: 2h30m\nsteps:\n  - id: notify\n    action: send_message\n    text: hello\n`,
];

test("accepted Form fixtures survive a semantic YAML round trip", () => {
  for (const fixture of acceptedFixtures) {
    const generated = formStateToYaml(accepted(fixture));
    assert.deepEqual(
      normalizeBackendDefaults(parseYaml(generated)),
      normalizeBackendDefaults(parseYaml(fixture)),
    );
  }
});

test("recognized nodes with unknown fields are refused without touching YAML", () => {
  const fixtures = [
    `name: Test\nunknown: true\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Test\ntrigger: { on: message_posted, future_filter: x }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: hi, retry: 3 }]\n`,
    `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: call_webhook, url: https://example.com, auth: bearer }]\n`,
  ];

  for (const yaml of fixtures) {
    const original = yaml;
    const result = yamlToFormState(yaml);
    assert.equal(result.ok, false);
    assert.match(result.error, /YAML editor/);
    assert.equal(yaml, original);
  }
});

test("invalid IDs, shapes, and scalar types are refused", () => {
  const cases = [
    [
      "missing ID",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ action: send_message, text: hi }]\n`,
    ],
    [
      "duplicate ID",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: same, action: send_message, text: hi }, { id: same, action: delay, duration: 5m }]\n`,
    ],
    [
      "invalid ID",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: bad-id, action: send_message, text: hi }]\n`,
    ],
    [
      "oversize ID",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: ${"a".repeat(65)}, action: send_message, text: hi }]\n`,
    ],
    [
      "steps object",
      `name: Test\ntrigger: { on: webhook }\nsteps: { id: s1, action: send_message, text: hi }\n`,
    ],
    [
      "missing required action field",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message }]\n`,
    ],
    [
      "numeric text",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: 42 }]\n`,
    ],
    [
      "numeric header",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: call_webhook, url: https://example.com, headers: { X-Retry: 3 } }]\n`,
    ],
    [
      "zero timeout",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, timeout_secs: 0, action: send_message, text: hi }]\n`,
    ],
    [
      "fractional timeout",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, timeout_secs: 1.5, action: send_message, text: hi }]\n`,
    ],
    [
      "unsupported method",
      `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: call_webhook, url: https://example.com, method: OPTIONS }]\n`,
    ],
  ];

  for (const [name, yaml] of cases) {
    assert.equal(yamlToFormState(yaml).ok, false, name);
  }
});

test("step condition capabilities stay in YAML mode", () => {
  const condition = `name: Conditional\ntrigger: { on: webhook }\nsteps: [{ id: s1, if: trigger_author == "abc", action: send_message, text: hi }]\n`;

  const conditionResult = yamlToFormState(condition);
  assert.equal(conditionResult.ok, false);
  assert.match(conditionResult.error, /conditions.*YAML editor/);
});

test("malformed and unowned schedule definitions stay losslessly in YAML mode", () => {
  const fixtures = [
    `name: Missing\ntrigger: { on: schedule }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Both\ntrigger: { on: schedule, cron: "0 9 * * *", interval: 1h }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Numeric\ntrigger: { on: schedule, interval: 30 }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Unknown\ntrigger: { on: schedule, cron: "0 9 * * *", timezone: UTC }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Invalid cron\ntrigger: { on: schedule, cron: "60 9 * * *" }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
  ];

  for (const yaml of fixtures) {
    const original = yaml;
    const result = yamlToFormState(yaml);
    assert.equal(result.ok, false);
    assert.match(result.error, /YAML editor/);
    assert.equal(yaml, original);
  }
});

test("the serializer emits only one schedule representation", () => {
  const yaml = formStateToYaml({
    name: "Exclusive",
    description: "",
    enabled: true,
    trigger: { on: "schedule", cron: "0 9 * * *", interval: "1h" },
    steps: [{ id: "s1", action: "send_message", text: "hi" }],
  });
  assert.deepEqual(parseYaml(yaml).trigger, {
    on: "schedule",
    cron: "0 9 * * *",
  });
});

test("presents step timeout seconds as durations and serializes them numerically", () => {
  const yaml = `name: Timed\ntrigger: { on: webhook }\nsteps: [{ id: s1, timeout_secs: 3602, action: send_message, text: hi }]\n`;
  const state = accepted(yaml);

  assert.equal(state.steps[0].timeoutSecs, "1h 2s");
  state.steps[0].timeoutSecs = "5m";
  assert.equal(parseYaml(formStateToYaml(state)).steps[0].timeout_secs, 300);
});

test("advanced message expressions survive unrelated Form serialization", () => {
  const filter =
    'str_contains(trigger_text, "deploy") && trigger_author == "abc"';
  const yaml = `name: Advanced\ndescription: Before\ntrigger:\n  on: message_posted\n  filter: '${filter}'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n`;
  const state = accepted(yaml);
  state.description = "After";
  const generated = parseYaml(formStateToYaml(state));

  assert.equal(generated.description, "After");
  assert.equal(generated.trigger.filter, filter);
});

test("values the Form serializer would normalize are refused", () => {
  const fixtures = [
    `name: Test\ndescription: " spaced "\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Test\ndescription: ""\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Test\ntrigger: { on: reaction_added, emoji: "" }\nsteps: [{ id: s1, action: send_message, text: hi }]\n`,
    `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, name: " spaced ", action: send_message, text: hi }]\n`,
    `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: send_message, text: hi, channel: "" }]\n`,
    `name: Test\ntrigger: { on: webhook }\nsteps: [{ id: s1, action: call_webhook, url: https://example.com, headers: { " padded ": value } }]\n`,
  ];

  for (const yaml of fixtures) assert.equal(yamlToFormState(yaml).ok, false);
});

test("reply_in_thread is emitted only when the checkbox is on", () => {
  const withReply = formStateToYaml(sendMessageState({ replyInThread: true }));
  assert.match(withReply, /reply_in_thread: true/);

  const withoutReply = formStateToYaml(
    sendMessageState({ replyInThread: false }),
  );
  assert.doesNotMatch(withoutReply, /reply_in_thread/);

  const unset = formStateToYaml(sendMessageState({}));
  assert.doesNotMatch(unset, /reply_in_thread/);
});

test("switching from Message Posted clears reply_in_thread before save", () => {
  const messagePosted = sendMessageState({ replyInThread: true });

  for (const triggerType of ["schedule", "webhook"]) {
    const switched = withTriggerType(messagePosted, triggerType);
    assert.equal(switched.trigger.on, triggerType);
    assert.equal(switched.steps[0].replyInThread, false);
    assert.doesNotMatch(formStateToYaml(switched), /reply_in_thread/);
  }
});

test("an ineligible trigger cannot resurrect reply_in_thread through an action change", () => {
  // Full repro: Message Posted → Send Message → enable Reply → switch action to
  // Delay → switch trigger to an ineligible one → switch action back to Send
  // Message. The action picker changes only `action` (a plain spread, mirrored
  // here), so `withTriggerType` must clear the hidden flag on every step, not
  // just the ones whose current action is send_message.
  for (const triggerType of ["schedule", "webhook"]) {
    const enabled = sendMessageState({ replyInThread: true });
    const asDelay = {
      ...enabled,
      steps: [{ ...enabled.steps[0], action: "delay", duration: "5m" }],
    };
    const switched = withTriggerType(asDelay, triggerType);
    const backToSend = {
      ...switched,
      steps: [{ ...switched.steps[0], action: "send_message" }],
    };

    assert.equal(backToSend.steps[0].replyInThread, false, triggerType);
    assert.doesNotMatch(formStateToYaml(backToSend), /reply_in_thread/);
  }
});

test("invalid reply_in_thread values are refused rather than normalized", () => {
  const original = (yaml) => {
    const result = yamlToFormState(yaml);
    assert.equal(result.ok, false);
    assert.match(result.error, /YAML editor/);
    return result;
  };

  // Non-boolean would be silently deleted on serialization.
  const nonBoolean = `name: Coerced\ntrigger: { on: message_posted }\nsteps: [{ id: s1, action: send_message, text: hi, reply_in_thread: "yes" }]\n`;
  assert.match(original(nonBoolean).error, /reply_in_thread must be a boolean/);

  // true under an ineligible trigger would round-trip a backend-invalid definition.
  for (const trigger of ["schedule, cron: '0 9 * * *'", "webhook"]) {
    const yaml = `name: Ineligible\ntrigger: { on: ${trigger} }\nsteps: [{ id: s1, action: send_message, text: hi, reply_in_thread: true }]\n`;
    assert.match(
      original(yaml).error,
      /reply_in_thread is not supported for (schedule|webhook) triggers/,
    );
  }
});

test("reply_in_thread eligibility follows trigger capability", () => {
  assert.equal(isThreadReplyEligibleTrigger("message_posted"), true);
  assert.equal(isThreadReplyEligibleTrigger("schedule"), false);
  assert.equal(isThreadReplyEligibleTrigger("webhook"), false);
});
test("reply_in_thread round-trips YAML -> form -> YAML", () => {
  const yaml = formStateToYaml(sendMessageState({ replyInThread: true }));
  const parsed = yamlToFormState(yaml);
  assert.equal(parsed.ok, true);
  assert.equal(parsed.state.steps[0].replyInThread, true);

  const reserialized = formStateToYaml(parsed.state);
  assert.match(reserialized, /reply_in_thread: true/);
});

test("absent reply_in_thread parses as false", () => {
  const yaml = [
    "name: No Reply",
    "trigger:",
    "  on: message_posted",
    "steps:",
    "  - id: step_1",
    "    action: send_message",
    "    text: hi",
    "",
  ].join("\n");
  const parsed = yamlToFormState(yaml);
  assert.equal(parsed.ok, true);
  assert.equal(parsed.state.steps[0].replyInThread, false);
});
