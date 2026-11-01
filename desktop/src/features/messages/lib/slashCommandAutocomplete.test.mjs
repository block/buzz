import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildSlashCommandGroups,
  buildSlashCommandInsertText,
  detectSlashCommandQuery,
  resolveLeadingAgentMentionPubkeys,
} from "./slashCommandAutocomplete.ts";

const ALPHA = "aa".repeat(32);
const BETA = "bb".repeat(32);
const catalog = new Map([
  [
    ALPHA,
    {
      seq: 1,
      timestamp: "2026-07-23T08:00:00Z",
      commands: [
        { name: "review", description: "Review the current changes" },
        { name: "deploy", description: "Ship to production" },
      ],
    },
  ],
  [
    BETA,
    {
      seq: 1,
      timestamp: "2026-07-23T08:00:00Z",
      commands: [{ name: "review", description: "Independent review" }],
    },
  ],
]);
const providers = [
  { pubkey: ALPHA, displayName: "Alpha" },
  { pubkey: BETA, displayName: "Beta" },
];

describe("slash command autocomplete", () => {
  it("detects a slash at message start and after leading agent mentions", () => {
    assert.deepEqual(detectSlashCommandQuery("/rev", 4), {
      leadingText: "",
      query: "rev",
      replaceFromOffset: 0,
    });
    assert.deepEqual(detectSlashCommandQuery("@Alpha /dep", 11), {
      leadingText: "@Alpha ",
      query: "dep",
      replaceFromOffset: 7,
    });
  });

  it("resolves selected or manually typed leading member-agent mentions", () => {
    assert.deepEqual(
      resolveLeadingAgentMentionPubkeys("@Alpha ", [
        { displayName: "Alpha", pubkey: ALPHA },
      ]),
      [ALPHA],
    );
    assert.deepEqual(
      resolveLeadingAgentMentionPubkeys("@Alpha @Beta ", [
        { displayName: "Alpha", pubkey: ALPHA },
        { displayName: "Beta", pubkey: BETA },
      ]),
      [ALPHA, BETA],
    );
    assert.deepEqual(
      resolveLeadingAgentMentionPubkeys("@Alpha hello ", [
        { displayName: "Alpha", pubkey: ALPHA },
      ]),
      [],
    );
  });

  it("does not trigger for inline paths, arguments, or multi-line text", () => {
    assert.equal(detectSlashCommandQuery("please /review", 14), null);
    assert.equal(detectSlashCommandQuery("/review now", 11), null);
    assert.equal(detectSlashCommandQuery("hello\n/review", 13), null);
  });

  it("routes commands chosen at message start through the provider mention", () => {
    const [group] = buildSlashCommandGroups({
      catalog,
      providers: [providers[0]],
      query: "rev",
      selectedAgentPubkeys: null,
    });
    const [suggestion] = group.commands;

    assert.equal(
      buildSlashCommandInsertText(suggestion, false),
      "@Alpha /review ",
    );
    assert.equal(buildSlashCommandInsertText(suggestion, true), "/review ");
  });

  it("groups duplicate command names by provider and narrows to mentions", () => {
    const all = buildSlashCommandGroups({
      catalog,
      providers,
      query: "rev",
      selectedAgentPubkeys: null,
    });
    assert.deepEqual(
      all.map((group) => [group.agentDisplayName, group.commands[0].name]),
      [
        ["Alpha", "review"],
        ["Beta", "review"],
      ],
    );

    const mentioned = buildSlashCommandGroups({
      catalog,
      providers,
      query: "",
      selectedAgentPubkeys: [BETA],
    });
    assert.deepEqual(
      mentioned.map((group) => group.agentPubkey),
      [BETA],
    );
  });

  it("ranks name prefixes before infix and description matches", () => {
    const rankedCatalog = new Map([
      [
        ALPHA,
        {
          seq: 1,
          timestamp: "2026-07-23T08:00:00Z",
          commands: [
            { name: "preview", description: null },
            { name: "review", description: null },
            { name: "inspect", description: "review changes" },
          ],
        },
      ],
    ]);
    const [group] = buildSlashCommandGroups({
      catalog: rankedCatalog,
      providers: [providers[0]],
      query: "rev",
      selectedAgentPubkeys: null,
    });
    assert.deepEqual(
      group.commands.map((command) => command.name),
      ["review", "preview", "inspect"],
    );
  });
});
