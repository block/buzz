import assert from "node:assert/strict";
import test from "node:test";
import { parse as parseYaml } from "yaml";

import {
  formStateToYaml,
  supportsMessageTextCondition,
  yamlToFormState,
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
