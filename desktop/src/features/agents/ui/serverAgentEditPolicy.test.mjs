import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveServerAgentEditContext,
  serverModelOptions,
} from "./serverAgentEditPolicy.ts";

test("relocated agent resolves to server context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner123",
    deployedSpawnerPubkey: null,
    agentPubkey: "agent456",
    slug: "my-agent",
    spawnerNameFor: () => "My Spawner",
  });

  assert.deepEqual(context, {
    spawnerPubkey: "spawner123",
    specSlug: "my-agent",
    agentPubkey: "agent456",
    spawnerName: "My Spawner",
  });
});

test("deployed agent resolves to server context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: null,
    deployedSpawnerPubkey: "spawner456",
    agentPubkey: "agent789",
    slug: "other-agent",
    spawnerNameFor: () => "Other Spawner",
  });

  assert.deepEqual(context, {
    spawnerPubkey: "spawner456",
    specSlug: "other-agent",
    agentPubkey: "agent789",
    spawnerName: "Other Spawner",
  });
});

test("relocated agent takes precedence over deployed spawner", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner-relocated",
    deployedSpawnerPubkey: "spawner-deployed",
    agentPubkey: "agent-id",
    slug: "test-slug",
    spawnerNameFor: (pubkey) =>
      pubkey === "spawner-relocated" ? "Relocated" : "Deployed",
  });

  assert.deepEqual(context, {
    spawnerPubkey: "spawner-relocated",
    specSlug: "test-slug",
    agentPubkey: "agent-id",
    spawnerName: "Relocated",
  });
});

test("local agent (no spawner context) returns null", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: null,
    deployedSpawnerPubkey: null,
    agentPubkey: "agent-local",
    slug: "local-agent",
    spawnerNameFor: () => "Never Called",
  });

  assert.equal(context, null);
});

test("empty relocatedToSpawner string is treated as null", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "",
    deployedSpawnerPubkey: null,
    agentPubkey: "agent-id",
    slug: "slug",
    spawnerNameFor: () => "Spawner",
  });

  assert.equal(context, null);
});

test("missing agentPubkey returns null even with spawner context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner123",
    deployedSpawnerPubkey: null,
    agentPubkey: null,
    slug: "slug",
    spawnerNameFor: () => "Spawner",
  });

  assert.equal(context, null);
});

test("empty agentPubkey returns null even with spawner context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner123",
    deployedSpawnerPubkey: null,
    agentPubkey: "",
    slug: "slug",
    spawnerNameFor: () => "Spawner",
  });

  assert.equal(context, null);
});

test("missing slug returns null even with spawner context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner123",
    deployedSpawnerPubkey: null,
    agentPubkey: "agent-id",
    slug: null,
    spawnerNameFor: () => "Spawner",
  });

  assert.equal(context, null);
});

test("empty slug returns null even with spawner context", () => {
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: "spawner123",
    deployedSpawnerPubkey: null,
    agentPubkey: "agent-id",
    slug: "",
    spawnerNameFor: () => "Spawner",
  });

  assert.equal(context, null);
});

test("serverModelOptions returns null when ai is undefined", () => {
  const result = serverModelOptions(undefined, "some-provider");
  assert.equal(result, null);
});

test("serverModelOptions returns null when ai is empty array", () => {
  const result = serverModelOptions([], "some-provider");
  assert.equal(result, null);
});

test("serverModelOptions returns all providers and selected provider's models", () => {
  const ai = [
    { id: "anthropic", models: ["claude-opus", "claude-sonnet"] },
    { id: "openai", models: ["gpt-4", "gpt-3.5-turbo"] },
  ];

  const result = serverModelOptions(ai, "anthropic");

  assert.deepEqual(result, {
    providers: ["anthropic", "openai"],
    models: ["claude-opus", "claude-sonnet"],
  });
});

test("serverModelOptions uses first provider when provider not found", () => {
  const ai = [
    { id: "anthropic", models: ["claude-opus"] },
    { id: "openai", models: ["gpt-4"] },
  ];

  const result = serverModelOptions(ai, "nonexistent");

  assert.deepEqual(result, {
    providers: ["anthropic", "openai"],
    models: ["claude-opus"],
  });
});

test("serverModelOptions uses first provider when provider is null", () => {
  const ai = [
    { id: "anthropic", models: ["claude-opus"] },
    { id: "openai", models: ["gpt-4"] },
  ];

  const result = serverModelOptions(ai, null);

  assert.deepEqual(result, {
    providers: ["anthropic", "openai"],
    models: ["claude-opus"],
  });
});

test("serverModelOptions handles single provider", () => {
  const ai = [{ id: "databricks", models: ["llama-2", "llama-3"] }];

  const result = serverModelOptions(ai, "databricks");

  assert.deepEqual(result, {
    providers: ["databricks"],
    models: ["llama-2", "llama-3"],
  });
});

test("serverModelOptions handles empty models array", () => {
  const ai = [{ id: "anthropic", models: [] }];

  const result = serverModelOptions(ai, "anthropic");

  assert.deepEqual(result, {
    providers: ["anthropic"],
    models: [],
  });
});
