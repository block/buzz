import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveReviewerWaits,
  mergeWorkstreamWaits,
  parseManualWorkstreamWaits,
  sortWorkstreamCards,
} from "./workstreamWaits.ts";

const LOGAN = "a".repeat(64);

test("manual waits retain simultaneous unique entries and ignore malformed or duplicate keys", () => {
  assert.deepEqual(
    parseManualWorkstreamWaits([
      {
        key: "log",
        actor: { kind: "person", pubkey: LOGAN, name: "Logan" },
        reason: "Decision",
      },
      {
        key: "review",
        actor: { kind: "team", name: "Reviewers" },
        reason: "Review",
      },
      {
        key: "review",
        actor: { kind: "team", name: "Duplicate" },
        reason: "Ignored",
      },
      { key: "bad", actor: { kind: "person", name: "" }, reason: "Invalid" },
    ]).map((wait) => wait.key),
    ["log", "review"],
  );
});

test("an open unapproved PR derives a wait which manual deletion cannot clear", () => {
  const reference = {
    kind: "github",
    identity: "github:block/buzz#1",
    rawUrl: "https://github.com/block/buzz/pull/1",
    href: "https://github.com/block/buzz/pull/1",
    owner: "block",
    repository: "buzz",
    number: 1,
  };
  const states = new Map([
    [
      reference.identity,
      {
        status: "live",
        provider: "github",
        href: reference.href,
        repository: "block/buzz",
        number: 1,
        title: "One",
        lifecycle: "open",
        reviewState: "review-requested",
      },
    ],
  ]);
  const derived = deriveReviewerWaits([reference], states);
  assert.equal(mergeWorkstreamWaits([], derived).length, 1);
  states.set(reference.identity, {
    ...states.get(reference.identity),
    lifecycle: "merged",
  });
  assert.equal(deriveReviewerWaits([reference], states).length, 0);
});

test("sorting authenticates the current owner by pubkey and remains stable across elapsed ticks", () => {
  const cards = [
    {
      channelId: "lookalike",
      channelName: "alpha",
      createdAt: "2026-08-18T02:00:00Z",
      waits: [
        {
          key: "name",
          actor: { kind: "person", name: "Logan" },
          reason: "Name only",
          source: "manual",
        },
      ],
    },
    {
      channelId: "active",
      channelName: "beta",
      createdAt: "2026-08-18T03:00:00Z",
      waits: [],
      activeWorking: {
        channelId: "active",
        agentCount: 1,
        agentPubkeys: ["b"],
        anchorAt: 1,
      },
    },
    {
      channelId: "owner",
      channelName: "zeta",
      createdAt: "2026-08-18T04:00:00Z",
      waits: [
        {
          key: "owner",
          actor: {
            kind: "person",
            name: "Not Logan",
            pubkey: LOGAN.toUpperCase(),
          },
          reason: "Decision",
          source: "manual",
        },
      ],
    },
  ];
  const ordered = sortWorkstreamCards(cards, LOGAN).map(
    (card) => card.channelId,
  );
  assert.deepEqual(ordered, ["owner", "active", "lookalike"]);
  assert.deepEqual(
    sortWorkstreamCards(
      cards.map((card) => ({
        ...card,
        activeWorking: card.activeWorking
          ? { ...card.activeWorking, anchorAt: 999999 }
          : undefined,
      })),
      LOGAN,
    ).map((card) => card.channelId),
    ordered,
  );
});

test("derived reviewer waits cover all unapproved open review states and exclude approved or terminal states", () => {
  const reference = {
    kind: "github",
    identity: "github:block/buzz#2",
    rawUrl: "https://github.com/block/buzz/pull/2",
    href: "https://github.com/block/buzz/pull/2",
    owner: "block",
    repository: "buzz",
    number: 2,
  };
  for (const reviewState of [
    "no-reviews",
    "review-requested",
    "changes-requested",
  ]) {
    const states = new Map([
      [
        reference.identity,
        {
          status: "live",
          provider: "github",
          href: reference.href,
          repository: "block/buzz",
          number: 2,
          title: "Two",
          lifecycle: "open",
          reviewState,
        },
      ],
    ]);
    assert.equal(
      deriveReviewerWaits([reference], states).length,
      1,
      reviewState,
    );
  }
  for (const lifecycle of ["draft", "merged", "closed"]) {
    const states = new Map([
      [
        reference.identity,
        {
          status: "live",
          provider: "github",
          href: reference.href,
          repository: "block/buzz",
          number: 2,
          title: "Two",
          lifecycle,
          reviewState: "review-requested",
        },
      ],
    ]);
    assert.equal(deriveReviewerWaits([reference], states).length, 0, lifecycle);
  }
  const approved = new Map([
    [
      reference.identity,
      {
        status: "live",
        provider: "github",
        href: reference.href,
        repository: "block/buzz",
        number: 2,
        title: "Two",
        lifecycle: "open",
        reviewState: "approved",
      },
    ],
  ]);
  assert.equal(deriveReviewerWaits([reference], approved).length, 0);
});

test("manual wording overlays an identical derived PR key while a deleted manual wait leaves the derived wait", () => {
  const derived = [
    {
      key: "github:block/buzz#4",
      actor: { kind: "team", name: "Reviewers" },
      reason: "Review requested",
      source: "derived",
    },
  ];
  const manual = [
    {
      key: "github:block/buzz#4",
      actor: { kind: "team", name: "Release reviewers" },
      reason: "Please review release notes",
      source: "manual",
    },
  ];
  assert.deepEqual(mergeWorkstreamWaits(manual, derived), manual);
  assert.deepEqual(mergeWorkstreamWaits([], derived), derived);
});
